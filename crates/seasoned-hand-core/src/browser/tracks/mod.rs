//! PostBrowserActionHook — Phase 1 story 1.16.
//!
//! Runs after every `browser_*` tool dispatch and captures two extra
//! browser representations alongside Track A (live noVNC, which is
//! already streamed from Phase 0 and needs no backend work):
//!
//! - **Track B** (DOM text snapshot): for `browser_view` we reuse the
//!   text the tool already returned; for every other `browser_*` tool
//!   we re-fetch the canonical view via `SandboxClient::browser_view`
//!   (the same accessor the Phase 0 tool now uses, so there is exactly
//!   one HTTP path per logical op). The captured payload becomes a Misc
//!   event keyed by `call_id`; story 1.14's inline-or-file_ref helper
//!   keeps the body small in the event row.
//!
//! - **Track C** (PNG screenshot): a 3 s-bounded screenshot call writes
//!   `/workspace/.tracks/<call_id>.png` via the sandbox host-volume API
//!   and emits a Misc event whose payload carries the file_ref.
//!
//! ### Spec divergence (intentional, documented)
//!
//! The story says "the Observation event's payload gains a
//! `dom_text_ref` field." The Phase 0 `Hook::post` signature passes
//! `output: &ToolOutput` — immutable — and the Observation event is
//! emitted by `EventEmittingHook` synchronously inside the dispatcher
//! loop. Mutating that event after the fact would require either a
//! `&mut ToolOutput` trait-signature change or a different ordering
//! contract. Both are larger refactors that the story doesn't budget.
//!
//! Instead this hook emits side-channel **Misc** events
//! `browser_track_b{call_id, dom_text_ref}` and
//! `browser_track_c{call_id, file_ref}`. Downstream consumers
//! (Verifier context builder in story 1.9; frontend BrowserTab in
//! story 1.19) join Track B/C to the Action+Observation pair by
//! `call_id` — exactly as they would have joined to a field nested in
//! the Observation payload. Functionally equivalent, smaller blast
//! radius.
//!
//! refs: /specs/phase-1/architecture.md §2.7, §3.4
//! refs: /specs/phase-1/stories/story-1.16.md

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::dispatch::hooks::Hook;
use crate::events::truncation::write_large_or_inline;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::sandbox::SandboxError;
use crate::tools::{ToolContext, ToolError, ToolOutput};

/// Default screenshot capture budget (architecture §7 cost note).
pub const DEFAULT_SCREENSHOT_TIMEOUT: Duration = Duration::from_millis(3000);

pub struct PostBrowserActionHook {
    events: Arc<SqliteEventStore>,
    screenshot_timeout: Duration,
}

impl PostBrowserActionHook {
    pub fn new(events: Arc<SqliteEventStore>) -> Self {
        Self {
            events,
            screenshot_timeout: DEFAULT_SCREENSHOT_TIMEOUT,
        }
    }

    pub fn with_screenshot_timeout(mut self, timeout: Duration) -> Self {
        self.screenshot_timeout = timeout;
        self
    }

    async fn emit_misc(&self, session_id: &str, kind: &str, data: Value) {
        let mut payload = data;
        if let Value::Object(map) = &mut payload {
            map.insert("kind".into(), Value::String(kind.into()));
        }
        if let Err(error) = self
            .events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "hook:browser_tracks".into(),
                data: payload,
            })
            .await
        {
            tracing::warn!(%error, kind, "browser_tracks misc append failed");
        }
    }
}

#[async_trait]
impl Hook for PostBrowserActionHook {
    async fn pre(&self, _: &str, _: &str, _: &Value, _: &ToolContext) {}

    async fn post(
        &self,
        call_id: &str,
        name: &str,
        _args: &Value,
        output: &ToolOutput,
        ctx: &ToolContext,
    ) {
        if !name.starts_with("browser_") {
            return;
        }

        capture_track_b(self, call_id, name, output, ctx).await;
        capture_track_c(self, call_id, ctx).await;
    }

    async fn failure(&self, _: &str, _: &str, _: &Value, _: &ToolError, _: &ToolContext) {}
}

async fn capture_track_b(
    hook: &PostBrowserActionHook,
    call_id: &str,
    tool_name: &str,
    output: &ToolOutput,
    ctx: &ToolContext,
) {
    let dom_text = if tool_name == "browser_view" {
        match dom_text_from_browser_view_output(&output.output) {
            Some(text) => text,
            None => {
                hook.emit_misc(
                    &ctx.session_id,
                    "browser_track_b_skipped",
                    json!({"call_id": call_id, "reason": "browser_view_payload_missing_elements"}),
                )
                .await;
                return;
            }
        }
    } else {
        match ctx.sandbox.browser_view(&ctx.session_id).await {
            Ok(view) => match dom_text_from_browser_view_output(&view) {
                Some(text) => text,
                None => {
                    hook.emit_misc(
                        &ctx.session_id,
                        "browser_track_b_skipped",
                        json!({"call_id": call_id, "reason": "browser_view_payload_missing_elements"}),
                    )
                    .await;
                    return;
                }
            },
            Err(err) => {
                hook.emit_misc(
                    &ctx.session_id,
                    "browser_track_b_skipped",
                    json!({"call_id": call_id, "reason": skip_reason_for(&err)}),
                )
                .await;
                return;
            }
        }
    };

    let next_id = match hook.events.reserve_next_id().await {
        Ok(id) => id,
        Err(error) => {
            tracing::warn!(%error, "browser_tracks: reserve_next_id failed");
            return;
        }
    };
    let dom_text_ref = match write_large_or_inline(
        &ctx.sandbox,
        &ctx.session_id,
        next_id,
        dom_text.as_bytes(),
        "text/plain",
    )
    .await
    {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!(%error, "browser_tracks: dom_text persistence failed");
            hook.emit_misc(
                &ctx.session_id,
                "browser_track_b_skipped",
                json!({"call_id": call_id, "reason": "dom_text_persist_failed"}),
            )
            .await;
            return;
        }
    };

    hook.emit_misc(
        &ctx.session_id,
        "browser_track_b",
        json!({
            "call_id": call_id,
            "dom_text_ref": dom_text_ref,
        }),
    )
    .await;
}

async fn capture_track_c(hook: &PostBrowserActionHook, call_id: &str, ctx: &ToolContext) {
    let fetch = ctx.sandbox.browser_screenshot(&ctx.session_id);
    let png = match tokio::time::timeout(hook.screenshot_timeout, fetch).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(err)) => {
            hook.emit_misc(
                &ctx.session_id,
                "browser_track_c_skipped",
                json!({"call_id": call_id, "reason": skip_reason_for(&err)}),
            )
            .await;
            return;
        }
        Err(_) => {
            hook.emit_misc(
                &ctx.session_id,
                "browser_track_c_skipped",
                json!({"call_id": call_id, "reason": "timeout"}),
            )
            .await;
            return;
        }
    };

    let path = format!("/workspace/.tracks/{call_id}.png");
    if let Err(error) = ctx
        .sandbox
        .write_workspace_file(&ctx.session_id, &path, &png)
        .await
    {
        tracing::warn!(%error, "browser_tracks: screenshot write failed");
        hook.emit_misc(
            &ctx.session_id,
            "browser_track_c_skipped",
            json!({"call_id": call_id, "reason": "sandbox_write_failed"}),
        )
        .await;
        return;
    }

    let sha256 = format!("{:x}", Sha256::digest(&png));
    hook.emit_misc(
        &ctx.session_id,
        "browser_track_c",
        json!({
            "call_id": call_id,
            "file_ref": {
                "path": path,
                "sha256": sha256,
                "size": png.len() as u64,
                "content_type": "image/png",
            },
        }),
    )
    .await;
}

/// Pull a flat text representation from a `browser_view` payload. The
/// AIO Sandbox `/v1/browser/page/elements` endpoint returns a tree of
/// element descriptors; if upstream ever exposes a `text` field we
/// prefer it, otherwise we serialize the elements blob as JSON. Both
/// shapes are downstream-usable by the Verifier; the inline-vs-file_ref
/// switch is handled by story 1.14's helper.
fn dom_text_from_browser_view_output(output: &Value) -> Option<String> {
    if let Some(text) = output.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    let elements = output.get("elements")?;
    if let Some(text) = elements.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    serde_json::to_string(elements).ok()
}

fn skip_reason_for(err: &SandboxError) -> String {
    match err {
        SandboxError::NotFound(_) => "sandbox_not_ready".into(),
        SandboxError::Http(_) => "sandbox_http".into(),
        SandboxError::HttpStatus { status, .. } => format!("sandbox_http_{status}"),
        _ => "sandbox_error".into(),
    }
}

#[cfg(test)]
mod tests;
