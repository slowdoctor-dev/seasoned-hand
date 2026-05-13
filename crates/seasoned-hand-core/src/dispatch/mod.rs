//! Tool dispatcher: registry-based name resolution + hook fan-out.
//! refs: /specs/phase-0/architecture.md §4.3

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};
use uuid::Uuid;

use crate::dispatch::mask::{DefaultMaskPolicy, ToolMaskPolicy};
use crate::events::{EventStore, EventType, NewEvent};
use crate::tools::{Tool, ToolContext, ToolError, ToolErrorPayload, ToolOutput};

pub mod hooks;
pub mod mask;

pub struct ToolDispatcher {
    registry: HashMap<&'static str, Arc<dyn Tool>>,
    hooks: Vec<Arc<dyn hooks::Hook>>,
    mask_policy: Arc<dyn ToolMaskPolicy>,
}

impl ToolDispatcher {
    pub fn new(registry: HashMap<&'static str, Arc<dyn Tool>>) -> Self {
        Self {
            registry,
            hooks: Vec::new(),
            mask_policy: Arc::new(DefaultMaskPolicy),
        }
    }

    pub fn with_hook(mut self, hook: Arc<dyn hooks::Hook>) -> Self {
        self.hooks.push(hook);
        self
    }

    pub fn registry(&self) -> &HashMap<&'static str, Arc<dyn Tool>> {
        &self.registry
    }

    /// Dispatch a tool call. Always returns a `ToolOutput` — internal
    /// errors are translated into `ok:false + error{kind}` payloads so
    /// the agent loop never has to catch a Rust-level error.
    pub async fn dispatch(&self, ctx: &ToolContext, tool_name: &str, args: Value) -> ToolOutput {
        if !self.mask_policy.is_available(tool_name, ctx.mask_mode) {
            if let Err(error) = ctx
                .events
                .append(NewEvent {
                    session_id: ctx.session_id.clone(),
                    event_type: EventType::Misc,
                    source: "dispatcher".into(),
                    data: json!({
                        "kind": "tool_mask_violation",
                        "tool": tool_name,
                        "mode": format!("{:?}", ctx.mask_mode),
                    }),
                })
                .await
            {
                tracing::warn!(%error, "failed to emit tool_mask_violation event");
            }
            return ToolOutput {
                ok: false,
                output: Value::Null,
                file_ref: None,
                error: Some(ToolErrorPayload {
                    kind: "tool_unavailable_in_iteration".into(),
                    message: format!(
                        "tool '{}' unavailable in mode {:?}",
                        tool_name, ctx.mask_mode
                    ),
                }),
            };
        }

        let Some(tool) = self.registry.get(tool_name).cloned() else {
            return ToolOutput {
                ok: false,
                output: Value::Null,
                file_ref: None,
                error: Some(ToolErrorPayload {
                    kind: "unknown_tool".into(),
                    message: format!("tool '{tool_name}' not in registry"),
                }),
            };
        };

        let call_id = Uuid::new_v4().to_string();

        for hook in &self.hooks {
            hook.pre(&call_id, tool_name, &args, ctx).await;
        }

        let result = tool.invoke(args.clone(), ctx).await;

        match result {
            Ok(out) => {
                for hook in &self.hooks {
                    hook.post(&call_id, tool_name, &args, &out, ctx).await;
                }
                out
            }
            Err(err) => {
                for hook in &self.hooks {
                    hook.failure(&call_id, tool_name, &args, &err, ctx).await;
                }
                ToolOutput {
                    ok: false,
                    output: Value::Null,
                    file_ref: None,
                    error: Some(ToolErrorPayload {
                        kind: tool_error_kind(&err).into(),
                        message: err.to_string(),
                    }),
                }
            }
        }
    }
}

fn tool_error_kind(err: &ToolError) -> &'static str {
    match err {
        ToolError::InvalidArgs(_) => "invalid_args",
        ToolError::Backend(_) => "backend",
        ToolError::NotImplemented(_) => "not_implemented",
        ToolError::Internal(_) => "internal",
    }
}

#[cfg(test)]
mod tests;
