//! Built-in tools for Phase 0.
//! Five backend-free tools: message_notify_user, message_ask_user, idle,
//! sop_read (stub), glossary_lookup (stub).
//!
//! refs: /specs/phase-0/architecture.md §4.3

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Tool, ToolContext, ToolError, ToolErrorPayload, ToolOutput};
use crate::events::{EventStore, EventType, NewEvent};

fn require_str(args: &Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing '{key}' string")))
}

fn deferred(phase: &'static str) -> ToolOutput {
    ToolOutput {
        ok: false,
        output: Value::Null,
        file_ref: None,
        error: Some(ToolErrorPayload {
            kind: "not_implemented".into(),
            message: format!("deferred to phase {phase}"),
        }),
    }
}

// ===== message_notify_user =====

pub struct MessageNotifyUser;

#[async_trait]
impl Tool for MessageNotifyUser {
    fn name(&self) -> &'static str {
        "message_notify_user"
    }
    fn description(&self) -> &'static str {
        "Send an informational message to the user. Fire-and-forget; the agent loop does not wait for a reply."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "Message body shown to the user." }
            },
            "required": ["content"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let content = require_str(&args, "content")?;
        let event = ctx
            .events
            .append(NewEvent {
                session_id: ctx.session_id.clone(),
                event_type: EventType::Message,
                source: format!("tool:{}", self.name()),
                data: json!({"role": "assistant", "content": content, "ui": "notify"}),
            })
            .await
            .map_err(|e| ToolError::Backend(e.to_string()))?;
        Ok(ToolOutput {
            ok: true,
            output: json!({"event_id": event.id}),
            file_ref: None,
            error: None,
        })
    }
}

// ===== message_ask_user =====

pub struct MessageAskUser;

#[async_trait]
impl Tool for MessageAskUser {
    fn name(&self) -> &'static str {
        "message_ask_user"
    }
    fn description(&self) -> &'static str {
        "Ask the user a question and block the agent loop until a reply arrives via the WebSocket user_response command."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "Question shown to the user." }
            },
            "required": ["content"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let content = require_str(&args, "content")?;
        let event = ctx
            .events
            .append(NewEvent {
                session_id: ctx.session_id.clone(),
                event_type: EventType::Message,
                source: format!("tool:{}", self.name()),
                data: json!({"role": "assistant", "content": content, "ui": "ask"}),
            })
            .await
            .map_err(|e| ToolError::Backend(e.to_string()))?;
        // Loop-blocking semantics land in story 0.14 (agent runner). Here we
        // just emit the event; the runner observes ui:"ask" and pauses.
        Ok(ToolOutput {
            ok: true,
            output: json!({"event_id": event.id, "call_id": event.id}),
            file_ref: None,
            error: None,
        })
    }
}

// ===== idle =====

pub struct Idle;

#[async_trait]
impl Tool for Idle {
    fn name(&self) -> &'static str {
        "idle"
    }
    fn description(&self) -> &'static str {
        "Signal that the task is complete and the agent loop should terminate."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            ok: true,
            output: json!({"signal": "task_complete"}),
            file_ref: None,
            error: None,
        })
    }
}

// ===== sop_read (stub) =====

pub struct SopRead;

#[async_trait]
impl Tool for SopRead {
    fn name(&self) -> &'static str {
        "sop_read"
    }
    fn description(&self) -> &'static str {
        "Look up a Standard Operating Procedure by id or title. (Deferred to Phase 3+; returns not_implemented.)"
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" }
            },
            "required": ["id"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        Ok(deferred("3"))
    }
}

// ===== glossary_lookup (stub) =====

pub struct GlossaryLookup;

#[async_trait]
impl Tool for GlossaryLookup {
    fn name(&self) -> &'static str {
        "glossary_lookup"
    }
    fn description(&self) -> &'static str {
        "Look up an organizational term in the glossary. (Deferred to Phase 3+; returns not_implemented.)"
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "term": { "type": "string" }
            },
            "required": ["term"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        Ok(deferred("3"))
    }
}

pub fn all() -> HashMap<&'static str, Arc<dyn Tool>> {
    let mut map: HashMap<&'static str, Arc<dyn Tool>> = HashMap::new();
    map.insert("message_notify_user", Arc::new(MessageNotifyUser));
    map.insert("message_ask_user", Arc::new(MessageAskUser));
    map.insert("idle", Arc::new(Idle));
    map.insert("sop_read", Arc::new(SopRead));
    map.insert("glossary_lookup", Arc::new(GlossaryLookup));
    map
}
