//! Tool catalog. Phase 0 ships 5 of the 32 tools defined in
//! architecture §4.3 + §7; the rest land in stories 0.7+.
//!
//! refs: /specs/phase-0/architecture.md §4.3, §7

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::checkpoint::{CheckpointLabelBuffer, CheckpointStore};
use crate::dispatch::mask::MaskContext;
use crate::events::sqlite::SqliteEventStore;
use crate::plan::PlanManager;
use crate::sandbox::SandboxClient;
use crate::search::SearchClient;

#[derive(Debug, Serialize)]
pub struct ToolOutput {
    pub ok: bool,
    pub output: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ToolErrorPayload>,
}

#[derive(Debug, Serialize)]
pub struct ToolErrorPayload {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    #[error("backend error: {0}")]
    Backend(String),
    #[error("not implemented (deferred to {0})")]
    NotImplemented(&'static str),
    #[error("internal: {0}")]
    Internal(String),
}

/// Context handed to every tool invocation.
///
/// Phase 0 carries `session_id` and the event store. Stories 0.7+ extend
/// this with `sandbox`, `search`, `deploy` backend handles (architecture
/// §4.3).
#[derive(Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub mask_ctx: MaskContext,
    pub events: Arc<SqliteEventStore>,
    pub sandbox: Arc<SandboxClient>,
    pub search: Arc<SearchClient>,
    pub plan_manager: Arc<PlanManager>,
    /// Story 1.13: one-shot label slot consumed by the next
    /// `Plan{op:"advance"}` checkpoint. The `checkpoint_label` tool
    /// writes here; the `CheckpointManager` reads + clears.
    pub checkpoint_labels: Arc<CheckpointLabelBuffer>,
    /// Story 1.13b: persistence handle for the `checkpoints` table —
    /// used by the internal `checkpoint_rollback` tool to read the
    /// target row, look up sibling rows, and stamp `rolled_back_at` /
    /// `rolled_back_by` after a successful `git revert`.
    pub checkpoints: Arc<CheckpointStore>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn schema(&self) -> Value;
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError>;
}

pub mod builtin;

/// Returns the 5 Phase-0 tools wired into a name → Arc registry.
pub fn register_builtin_tools() -> HashMap<&'static str, Arc<dyn Tool>> {
    builtin::all()
}

#[cfg(test)]
mod tests;
