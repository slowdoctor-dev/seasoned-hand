//! `RendererDispatcher` — given (source content, target filename) →
//! writes the rendered artifact into `/workspace/.deliverables/` and
//! returns a [`RenderedArtifact`] matching the [`crate::deliverable::Deliverable`]
//! column shape.
//!
//! Routing by `target_filename` extension (architecture §2.3):
//!
//! | Extension | Renderer | Module |
//! |---|---|---|
//! | `md`, `txt`, `json`, `csv` | raw pass-through | [`raw`] |
//! | `docx`, `pdf`, `html`, `odt` | Pandoc CLI in sandbox | [`pandoc`] |
//! | `pptx` | python-pptx via inline Python in sandbox | [`python_pptx`] |
//! | `xlsx` | openpyxl via inline Python in sandbox | [`openpyxl`] |
//!
//! Failure surface: [`RenderError::RendererFailed`] carries the
//! renderer name, exit code, captured stderr, and an `input_preview`
//! truncated to 200 chars. Story 2.14 (`task_deliver` LLM tool) reads
//! this to drive the "simplify and retry" path; the dispatcher
//! itself does NOT retry (architecture §8 reserves that for the
//! LLM-driven flow).
//!
//! refs: /specs/phase-2/architecture.md §2.3, §5, §7
//! refs: /specs/phase-2/stories/story-2.6.md

use std::sync::Arc;

use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

#[doc(hidden)]
pub use crate::sandbox::bootstrap::ShellExecOutcome;

use crate::sandbox::{SandboxClient, SandboxError};

pub mod openpyxl;
pub mod pandoc;
pub mod python_pptx;
pub mod raw;

/// Rendered output written into `/workspace/.deliverables/`. Matches
/// the V007 deliverable-row column projection (path / size / sha256)
/// so the caller can directly construct a `NewDeliverable` row from
/// this struct.
#[derive(Debug, Clone)]
pub struct RenderedArtifact {
    /// Workspace-relative path of the rendered file. Always lives
    /// under `/workspace/.deliverables/`; persisted as the sandbox-
    /// relative form.
    pub workspace_path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Error)]
pub enum RenderError {
    /// Renderer exited non-zero. `input_preview` is the first 200
    /// chars of the source content for diagnostics — full bytes live
    /// in the source file on the workspace.
    #[error("renderer {renderer} failed (exit={exit_code}): {stderr}")]
    RendererFailed {
        renderer: &'static str,
        exit_code: i32,
        stderr: String,
        input_preview: String,
    },
    #[error("unsupported extension: {0}")]
    UnsupportedExtension(String),
    #[error("missing extension on target filename: {0}")]
    MissingExtension(String),
    #[error("sandbox error: {0}")]
    Sandbox(#[from] SandboxError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Workspace dir all renderers write source + output beneath. Sandbox-
/// relative so [`SandboxClient::write_workspace_file`] resolves to
/// the host bind mount.
pub const DELIVERABLES_DIR: &str = ".deliverables";

/// Sub-dir for LLM-source content (markdown/JSON). The rendered
/// artifact lands one level up.
pub const SOURCE_SUBDIR: &str = ".deliverables/.source";

/// Number of chars of the source content surfaced in
/// [`RenderError::RendererFailed.input_preview`].
const PREVIEW_CHARS: usize = 200;

#[derive(Clone)]
pub struct RendererDispatcher {
    sandbox: Arc<SandboxClient>,
}

impl RendererDispatcher {
    pub fn new(sandbox: Arc<SandboxClient>) -> Self {
        Self { sandbox }
    }

    /// Inspect `target_filename`'s extension and dispatch. Returns the
    /// [`RenderedArtifact`] with workspace-relative `path` once the
    /// renderer succeeds.
    pub async fn render(
        &self,
        source_content: &[u8],
        target_filename: &str,
        session_id: &str,
    ) -> Result<RenderedArtifact, RenderError> {
        let ext = extract_extension(target_filename)?;
        let renderer = pick_renderer(&ext)?;
        let target_path = format!("{DELIVERABLES_DIR}/{target_filename}");
        // The renderers under us assume the deliverables dir exists.
        ensure_dir(&self.sandbox, session_id, DELIVERABLES_DIR).await?;
        match renderer {
            Renderer::Raw => {
                raw::render(&self.sandbox, session_id, source_content, &target_path).await
            }
            Renderer::Pandoc => {
                let source_ext = "md"; // markdown is the only Pandoc input the dispatcher uses
                let source_path = source_temp_path(source_ext);
                ensure_dir(&self.sandbox, session_id, SOURCE_SUBDIR).await?;
                pandoc::render(
                    &self.sandbox,
                    session_id,
                    source_content,
                    &source_path,
                    &target_path,
                    &ext,
                )
                .await
            }
            Renderer::PythonPptx => {
                let source_path = source_temp_path("json");
                ensure_dir(&self.sandbox, session_id, SOURCE_SUBDIR).await?;
                python_pptx::render(
                    &self.sandbox,
                    session_id,
                    source_content,
                    &source_path,
                    &target_path,
                )
                .await
            }
            Renderer::Openpyxl => {
                let source_path = source_temp_path("json");
                ensure_dir(&self.sandbox, session_id, SOURCE_SUBDIR).await?;
                openpyxl::render(
                    &self.sandbox,
                    session_id,
                    source_content,
                    &source_path,
                    &target_path,
                )
                .await
            }
        }
    }
}

/// Internal renderer tag — keeps the dispatch arm exhaustive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Renderer {
    Raw,
    Pandoc,
    PythonPptx,
    Openpyxl,
}

pub(crate) fn pick_renderer(ext: &str) -> Result<Renderer, RenderError> {
    match ext {
        "md" | "txt" | "json" | "csv" => Ok(Renderer::Raw),
        "docx" | "pdf" | "html" | "odt" => Ok(Renderer::Pandoc),
        "pptx" => Ok(Renderer::PythonPptx),
        "xlsx" => Ok(Renderer::Openpyxl),
        other => Err(RenderError::UnsupportedExtension(other.to_string())),
    }
}

fn extract_extension(target_filename: &str) -> Result<String, RenderError> {
    let lower = target_filename.to_ascii_lowercase();
    let Some(dot) = lower.rfind('.') else {
        return Err(RenderError::MissingExtension(target_filename.to_string()));
    };
    let ext = &lower[dot + 1..];
    if ext.is_empty() {
        return Err(RenderError::MissingExtension(target_filename.to_string()));
    }
    Ok(ext.to_string())
}

/// Make sure `dir` exists on the workspace before any renderer
/// writes. The bind mount means `tokio::fs::create_dir_all` from the
/// host side works equally well; we go through `mkdir -p` in the
/// sandbox so the file ends up with the sandbox-uid ownership the
/// container would have set anyway.
async fn ensure_dir(
    sandbox: &SandboxClient,
    session_id: &str,
    relative: &str,
) -> Result<(), SandboxError> {
    // Use the host-side mkdir — faster + doesn't require shell_exec.
    let handle = sandbox
        .get(session_id)
        .await
        .ok_or_else(|| SandboxError::NotFound(session_id.to_string()))?;
    let path = handle.workspace_host_path.join(relative);
    tokio::fs::create_dir_all(&path).await?;
    Ok(())
}

fn source_temp_path(ext: &str) -> String {
    format!("{SOURCE_SUBDIR}/{}.{ext}", Uuid::new_v4())
}

/// Read a rendered artifact off the workspace and compute its
/// `size` + `sha256` so the caller can persist a `Deliverable` row.
/// Shared by all 4 renderers.
pub(crate) async fn fingerprint_artifact(
    sandbox: &SandboxClient,
    session_id: &str,
    workspace_relative: &str,
) -> Result<RenderedArtifact, RenderError> {
    let bytes = sandbox
        .read_workspace_file(session_id, workspace_relative)
        .await?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    Ok(RenderedArtifact {
        workspace_path: workspace_relative.to_string(),
        size: bytes.len() as u64,
        sha256,
    })
}

/// Truncate source bytes to a printable preview for error surfaces.
pub(crate) fn preview_of(source: &[u8]) -> String {
    let s = String::from_utf8_lossy(source);
    if s.chars().count() <= PREVIEW_CHARS {
        s.into_owned()
    } else {
        s.chars().take(PREVIEW_CHARS).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests;
