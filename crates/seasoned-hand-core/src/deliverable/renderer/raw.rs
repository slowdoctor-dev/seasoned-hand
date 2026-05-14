//! Raw pass-through renderer for `md` / `txt` / `json` / `csv`.
//!
//! No transformation, no shell-exec — the LLM already produced the
//! deliverable text in the target format, so we just write the bytes
//! to the workspace and fingerprint.

use super::{RenderError, RenderedArtifact, fingerprint_artifact};
use crate::sandbox::SandboxClient;

pub async fn render(
    sandbox: &SandboxClient,
    session_id: &str,
    source_content: &[u8],
    target_path: &str,
) -> Result<RenderedArtifact, RenderError> {
    sandbox
        .write_workspace_file(session_id, target_path, source_content)
        .await?;
    fingerprint_artifact(sandbox, session_id, target_path).await
}
