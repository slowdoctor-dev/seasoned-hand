//! `openpyxl`-based renderer for `xlsx`.
//!
//! Writes the LLM-produced JSON (architecture §2.3 — `{sheets: [{name,
//! rows: [[...]], formats?}]}`) to a sandbox temp file and execs a
//! small Python script that materialises the workbook.

use super::{RenderError, RenderedArtifact, fingerprint_artifact, preview_of};
use crate::sandbox::SandboxClient;

pub const RENDERER_NAME: &str = "openpyxl";

const XLSX_SCRIPT: &str = r#"
import json, sys
from openpyxl import Workbook

src, dst = sys.argv[1], sys.argv[2]
with open(src) as f:
    doc = json.load(f)

wb = Workbook()
# openpyxl gives every Workbook a default sheet — drop it once we know
# the input has at least one named sheet.
default_dropped = False
for entry in doc.get("sheets", []):
    name = entry.get("name", "Sheet")
    ws = wb.create_sheet(name)
    if not default_dropped:
        del wb["Sheet"]
        default_dropped = True
    rows = entry.get("rows", [])
    for row in rows:
        ws.append(row)

wb.save(dst)
"#;

pub async fn render(
    sandbox: &SandboxClient,
    session_id: &str,
    source_content: &[u8],
    source_path: &str,
    target_path: &str,
) -> Result<RenderedArtifact, RenderError> {
    serde_json::from_slice::<serde_json::Value>(source_content)?;

    sandbox
        .write_workspace_file(session_id, source_path, source_content)
        .await?;
    let script_path = format!("{source_path}.xlsx.py");
    sandbox
        .write_workspace_file(session_id, &script_path, XLSX_SCRIPT.as_bytes())
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
