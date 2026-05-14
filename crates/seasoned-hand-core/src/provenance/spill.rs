//! Size-budgeted manifest persistence — inline up to 100 KB, spill to
//! `/workspace/.provenance/<task_id>.json` above that.
//!
//! Architecture §12 q5 fixes the 100 KB cap; story 2.15 implements it.
//!
//! refs: /specs/phase-2/architecture.md §2.11, §12 q5
//! refs: /specs/phase-2/stories/story-2.15.md

use serde_json::{Value, json};

use super::builder::ProvenanceError;
use super::manifest::ProvenanceManifest;
use crate::sandbox::SandboxClient;

/// Default inline manifest budget, 100 KB serialized JSON.
pub const INLINE_THRESHOLD_BYTES: usize = 100 * 1024;

/// Workspace path (sandbox-relative) where spilled manifests land.
fn workspace_path_for(task_id: &str) -> String {
    format!(".provenance/{task_id}.json")
}

/// File-URI form stored in the `deliverables.provenance_manifest`
/// column when a manifest spills. The route handler inspects the
/// `$ref` key to decide whether to load the file or use the value
/// directly.
fn file_uri_for(task_id: &str) -> String {
    format!("file:///workspace/.provenance/{task_id}.json")
}

/// Outcome of [`persist_or_spill`]. `Inline(Value)` carries the full
/// manifest JSON, ready to drop into [`crate::deliverable::NewDeliverable::provenance_manifest`].
/// `FileRef { ref_value, .. }` carries a `{"$ref": "file://..."}` Value
/// that goes into the same column; `workspace_path` is the path the
/// route handler reads back via [`SandboxClient::read_workspace_file`].
#[derive(Debug, Clone)]
pub enum ManifestColumn {
    Inline(Value),
    FileRef {
        workspace_path: String,
        ref_value: Value,
    },
}

impl ManifestColumn {
    /// Drop the variant tag — both arms produce the JSON `Value` that
    /// the V007 `provenance_manifest` column expects.
    pub fn into_column_value(self) -> Value {
        match self {
            ManifestColumn::Inline(v) => v,
            ManifestColumn::FileRef { ref_value, .. } => ref_value,
        }
    }

    /// True iff the manifest spilled to file.
    pub fn is_file_ref(&self) -> bool {
        matches!(self, ManifestColumn::FileRef { .. })
    }
}

/// Serialize the manifest; inline-encode if under `threshold` bytes,
/// otherwise write the JSON body to `/workspace/.provenance/<task>.json`
/// and return a file-ref `Value` for the column.
///
/// `threshold` is parameterized so tests can force the spill path
/// without inflating the manifest past 100 KB; production callers
/// should pass [`INLINE_THRESHOLD_BYTES`].
pub async fn persist_or_spill(
    manifest: &ProvenanceManifest,
    sandbox: &SandboxClient,
    session_id: &str,
    task_id: &str,
    threshold: usize,
) -> Result<ManifestColumn, ProvenanceError> {
    let serialized = serde_json::to_string(manifest)?;
    if serialized.len() <= threshold {
        return Ok(ManifestColumn::Inline(serde_json::to_value(manifest)?));
    }
    let path = workspace_path_for(task_id);
    sandbox
        .write_workspace_file(session_id, &path, serialized.as_bytes())
        .await?;
    let ref_value = json!({ "$ref": file_uri_for(task_id) });
    Ok(ManifestColumn::FileRef {
        workspace_path: path,
        ref_value,
    })
}

/// Extract the workspace-relative path from a `file:///workspace/<rel>`
/// URI. Returns `Err` if the URI is malformed or points outside
/// `/workspace`.
pub fn parse_workspace_uri(uri: &str) -> Result<String, ProvenanceError> {
    const PREFIX: &str = "file:///workspace/";
    uri.strip_prefix(PREFIX)
        .map(|rel| rel.to_string())
        .ok_or_else(|| ProvenanceError::InvalidFileRef(uri.to_string()))
}
