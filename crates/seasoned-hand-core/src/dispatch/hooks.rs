//! Pre/Post/PostFailure hook trait + EventEmittingHook (story 0.10).
//! refs: /specs/phase-0/architecture.md §4.3
//! refs: /specs/00-philosophy/PRINCIPLES.md #3 (append-only), #10 (failure-tolerant), #11 (audit trail)

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::events::payload::EventPayloadBody;
use crate::events::truncation::write_large_or_inline;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::tools::{ToolContext, ToolError, ToolOutput};

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

    async fn payload_for(
        &self,
        ctx: &ToolContext,
        body: &[u8],
        content_type: &str,
    ) -> Result<EventPayloadBody, crate::events::EventError> {
        let next_id = self.events.reserve_next_id().await?;
        write_large_or_inline(&ctx.sandbox, &ctx.session_id, next_id, body, content_type).await
    }
}

#[async_trait]
impl Hook for EventEmittingHook {
    async fn pre(&self, call_id: &str, name: &str, args: &Value, ctx: &ToolContext) {
        let body = match serde_json::to_vec(args) {
            Ok(body) => body,
            Err(e) => {
                tracing::warn!(error = %e, "hook args serialization failed");
                return;
            }
        };
        let payload = match self.payload_for(ctx, &body, "application/json").await {
            Ok(payload) => payload,
            Err(e) => {
                tracing::warn!(error = %e, "hook action payload capture failed");
                return;
            }
        };
        self.append_logged(
            NewEvent {
                session_id: ctx.session_id.clone(),
                event_type: EventType::Action,
                source: format!("tool:{name}"),
                data: json!({
                    "tool": name,
                    "body": payload,
                    "content_type": "application/json",
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
        let output_bytes = match serde_json::to_vec(&output.output) {
            Ok(body) => body,
            Err(e) => {
                tracing::warn!(error = %e, "hook output serialization failed");
                return;
            }
        };
        let payload = match self
            .payload_for(ctx, &output_bytes, "application/json")
            .await
        {
            Ok(payload) => payload,
            Err(e) => {
                tracing::warn!(error = %e, "hook observation payload capture failed");
                return;
            }
        };

        let mut data = json!({
            "call_id": call_id,
            "ok": output.ok,
            "body": payload,
            "content_type": "application/json",
        });
        if let Some(fr) = output.file_ref.as_deref() {
            data["file_ref"] = Value::String(fr.into());
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
