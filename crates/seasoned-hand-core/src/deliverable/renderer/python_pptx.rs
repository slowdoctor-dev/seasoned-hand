//! `python-pptx`-based renderer for `pptx`.
//!
//! Writes the LLM-produced JSON (architecture §2.3 — `{slides: [{title,
//! body, layout?}]}`) to a sandbox temp file and execs a small Python
//! script that materialises the deck.

use super::{RenderError, RenderedArtifact, fingerprint_artifact, preview_of};
use crate::sandbox::SandboxClient;

pub const RENDERER_NAME: &str = "python_pptx";

/// Inline Python script — reads source JSON from argv[1], writes the
/// pptx to argv[2]. Kept Rust-side so the renderer logic versions
/// with the crate rather than the sandbox image.
const PPTX_SCRIPT: &str = r#"
import json, sys
from pptx import Presentation

src, dst = sys.argv[1], sys.argv[2]
with open(src) as f:
    doc = json.load(f)

prs = Presentation()
title_layout = prs.slide_layouts[0]
content_layout = prs.slide_layouts[1]
slides = doc.get("slides", [])
for entry in slides:
    layout_name = entry.get("layout", "content")
    layout = title_layout if layout_name == "title" else content_layout
    slide = prs.slides.add_slide(layout)
    title = entry.get("title", "")
    body = entry.get("body", "")
    if slide.shapes.title is not None:
        slide.shapes.title.text = title
    for shape in slide.placeholders:
        if shape.placeholder_format.idx == 1:
            shape.text = body if isinstance(body, str) else "\n".join(body)
            break

prs.save(dst)
"#;

pub async fn render(
    sandbox: &SandboxClient,
    session_id: &str,
    source_content: &[u8],
    source_path: &str,
    target_path: &str,
) -> Result<RenderedArtifact, RenderError> {
    // Validate the JSON shape early so a malformed source surfaces as
    // RendererFailed (with the parse error in stderr) rather than as a
    // python traceback on the sandbox side.
    serde_json::from_slice::<serde_json::Value>(source_content)?;

    sandbox
        .write_workspace_file(session_id, source_path, source_content)
        .await?;
    let script_path = format!("{source_path}.pptx.py");
    sandbox
        .write_workspace_file(session_id, &script_path, PPTX_SCRIPT.as_bytes())
        .await?;

    let command = format!(
        "python3 /workspace/{script_path} /workspace/{source_path} /workspace/{target_path}"
    );
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
