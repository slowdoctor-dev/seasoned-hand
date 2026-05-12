//! Pre/Post/PostFailure hook trait. Bodies land in story 0.10.
//! refs: /specs/phase-0/architecture.md §4.3

use async_trait::async_trait;
use serde_json::Value;

use crate::tools::{ToolContext, ToolError, ToolOutput};

#[async_trait]
pub trait Hook: Send + Sync {
    async fn pre(&self, name: &str, args: &Value, ctx: &ToolContext);
    async fn post(&self, name: &str, args: &Value, output: &ToolOutput, ctx: &ToolContext);
    async fn failure(&self, name: &str, args: &Value, err: &ToolError, ctx: &ToolContext);
}

/// No-op hook used when nothing is registered.
pub struct NoopHook;

#[async_trait]
impl Hook for NoopHook {
    async fn pre(&self, _name: &str, _args: &Value, _ctx: &ToolContext) {}
    async fn post(&self, _name: &str, _args: &Value, _output: &ToolOutput, _ctx: &ToolContext) {}
    async fn failure(&self, _name: &str, _args: &Value, _err: &ToolError, _ctx: &ToolContext) {}
}
