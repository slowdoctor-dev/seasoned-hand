//! Pandoc-based renderer for `docx` / `pdf` / `html` / `odt`.
//!
//! Writes the LLM-produced markdown to a sandbox temp file, then
//! shell-execs `pandoc -f markdown -t <fmt> -o <target> <source>` to
//! produce the rendered artifact. PDF specifically goes through
//! `--pdf-engine=xelatex` (the texlive-xetex package installed at
//! sandbox-create time, story 2.6 install step).

use super::{RenderError, RenderedArtifact, fingerprint_artifact, preview_of};
use crate::sandbox::SandboxClient;

pub const RENDERER_NAME: &str = "pandoc";

pub async fn render(
    sandbox: &SandboxClient,
    session_id: &str,
    source_content: &[u8],
    source_path: &str,
    target_path: &str,
    target_ext: &str,
) -> Result<RenderedArtifact, RenderError> {
    sandbox
        .write_workspace_file(session_id, source_path, source_content)
        .await?;

    let command = pandoc_command(source_path, target_path, target_ext);
    let outcome = sandbox.shell_exec(session_id, &command).await?;
    if outcome.exit_code != 0 {
        return Err(RenderError::RendererFailed {
            renderer: RENDERER_NAME,
            exit_code: outcome.exit_code,
            stderr: outcome.stderr,
            input_preview: preview_of(source_content),
        });
    }
    fingerprint_artifact(sandbox, session_id, target_path).await
}

pub(crate) fn pandoc_command(source_path: &str, target_path: &str, target_ext: &str) -> String {
    // The /workspace prefix lands the path inside the container's bind
    // mount; the host-side `read_workspace_file` resolves the same
    // relative form.
    let source_in = format!("/workspace/{source_path}");
    let target_in = format!("/workspace/{target_path}");
    match target_ext {
        "pdf" => {
            format!("pandoc -f markdown -t pdf --pdf-engine=xelatex -o {target_in} {source_in}")
        }
        // pandoc accepts the ext directly as the format name for docx,
        // html, odt.
        ext => format!("pandoc -f markdown -t {ext} -o {target_in} {source_in}"),
    }
}
