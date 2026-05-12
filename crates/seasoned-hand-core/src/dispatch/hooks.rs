//! Pre/Post/PostFailure hook trait + EventEmittingHook (story 0.10).
//! refs: /specs/phase-0/architecture.md §4.3
//! refs: /specs/00-philosophy/PRINCIPLES.md #3 (append-only), #10 (failure-tolerant), #11 (audit trail)

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::tools::{ToolContext, ToolError, ToolOutput};

/// Inline output cap before the hook falls back to a truncation marker.
/// Architecture §3.4 says 16 KB; we serialize JSON, so 16 * 1024 bytes
/// of the JSON-encoded output triggers the truncation path.
pub const INLINE_OUTPUT_LIMIT: usize = 16 * 1024;
pub const TRUNCATION_PREVIEW: usize = 1024;

#[async_trait]
pub trait Hook: Send + Sync {
    async fn pre(&self, call_id: &str, name: &str, args: &Value, ctx: &ToolContext);
    async fn post(
        &self,
        call_id: &str,
        name: &str,
        args: &Value,
        output: &ToolOutput,
        ctx: &ToolContext,
    );
    async fn failure(
        &self,
        call_id: &str,
        name: &str,
        args: &Value,
        err: &ToolError,
        ctx: &ToolContext,
    );
}

/// No-op hook used when nothing is registered.
pub struct NoopHook;

#[async_trait]
impl Hook for NoopHook {
    async fn pre(&self, _: &str, _: &str, _: &Value, _: &ToolContext) {}
    async fn post(&self, _: &str, _: &str, _: &Value, _: &ToolOutput, _: &ToolContext) {}
    async fn failure(&self, _: &str, _: &str, _: &Value, _: &ToolError, _: &ToolContext) {}
}

/// Writes Action + Observation events to the event store for every
/// tool dispatch. The dispatcher generates a UUID `call_id` and threads
/// it through pre/post/failure so the Observation can be linked back
/// to its originating Action.
///
/// Append failures here are **logged, never propagated** — PRINCIPLE #10:
/// the tool dispatch itself must not be broken by an auxiliary failure
/// to record it.
pub struct EventEmittingHook {
    pub events: Arc<SqliteEventStore>,
}

impl EventEmittingHook {
    pub fn new(events: Arc<SqliteEventStore>) -> Self {
        Self { events }
    }

    async fn append_logged(&self, event: NewEvent, label: &'static str) {
        if let Err(e) = self.events.append(event).await {
            tracing::warn!(error = %e, hook = label, "hook event append failed");
        }
    }
}

#[async_trait]
impl Hook for EventEmittingHook {
    async fn pre(&self, call_id: &str, name: &str, args: &Value, ctx: &ToolContext) {
        self.append_logged(
            NewEvent {
                session_id: ctx.session_id.clone(),
                event_type: EventType::Action,
                source: format!("tool:{name}"),
                data: json!({
                    "tool": name,
                    "args": args,
                    "call_id": call_id,
                }),
            },
            "pre",
        )
        .await;
    }

    async fn post(
        &self,
        call_id: &str,
        name: &str,
        _args: &Value,
        output: &ToolOutput,
        ctx: &ToolContext,
    ) {
        let (recorded_output, file_ref, truncated) = downsize_output(&output.output);

        let mut data = json!({
            "call_id": call_id,
            "ok": output.ok,
            "output": recorded_output,
        });
        if let Some(fr) = output.file_ref.as_deref() {
            data["file_ref"] = Value::String(fr.into());
        } else if let Some(fr) = file_ref {
            data["file_ref"] = Value::String(fr);
        }
        if truncated {
            data["truncated"] = Value::Bool(true);
        }
        if let Some(err) = &output.error {
            data["error"] = json!({ "kind": err.kind, "message": err.message });
        }

        self.append_logged(
            NewEvent {
                session_id: ctx.session_id.clone(),
                event_type: EventType::Observation,
                source: format!("tool:{name}"),
                data,
            },
            "post",
        )
        .await;
    }

    async fn failure(
        &self,
        call_id: &str,
        name: &str,
        _args: &Value,
        err: &ToolError,
        ctx: &ToolContext,
    ) {
        self.append_logged(
            NewEvent {
                session_id: ctx.session_id.clone(),
                event_type: EventType::Observation,
                source: format!("tool:{name}"),
                data: json!({
                    "call_id": call_id,
                    "ok": false,
                    "error": { "kind": tool_error_kind(err), "message": err.to_string() },
                }),
            },
            "failure",
        )
        .await;
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

/// Architecture §3.4 oversize handling. Phase 0 stores the truncated
/// preview inline with a `truncated:true` marker rather than writing
/// to sandbox `/workspace/.observations/<call_id>.txt` — the sandbox
/// upload path lands when the broader sandbox-tool wiring (DEBT #19)
/// completes. Tracked as DEBT.md item #21.
fn downsize_output(output: &Value) -> (Value, Option<String>, bool) {
    let serialized = serde_json::to_string(output).unwrap_or_default();
    if serialized.len() <= INLINE_OUTPUT_LIMIT {
        return (output.clone(), None, false);
    }
    let preview = serialized
        .chars()
        .take(TRUNCATION_PREVIEW)
        .collect::<String>();
    (
        json!({ "preview": preview, "preview_chars": TRUNCATION_PREVIEW }),
        None,
        true,
    )
}
