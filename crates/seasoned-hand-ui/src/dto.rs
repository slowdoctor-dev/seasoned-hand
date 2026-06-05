//! Data shapes shared with the Rust control plane's `/v1` + `/ws` API.
//!
//! These mirror `crates/seasoned-hand-core` DTOs (and the legacy
//! `frontend/lib/api.ts` + `frontend/lib/ws-types.ts`). Per ADR-016 the
//! intended end-state is a wasm-safe `seasoned-hand-dto` workspace crate shared
//! by both the server and this UI; until that extraction lands these structs
//! are hand-mirrored, exactly as the TypeScript layer was.

use serde::{Deserialize, Serialize};

// Backend timestamps are integer epoch values (micros); kept as i64.
pub type Timestamp = i64;

// ----------------------------------------------------------------------------
// REST DTOs (mirror frontend/lib/api.ts)
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SessionState {
    Idle,
    Running,
    Finished,
    Error,
    Suspended,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub state: SessionState,
    pub title: Option<String>,
    pub cost_cents: i64,
    pub tool_calls: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStatus {
    Active,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub tenant_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub status: ProjectStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Drafted,
    Briefed,
    Confirmed,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub tenant_id: Option<String>,
    pub title: String,
    pub brief: Option<serde_json::Value>,
    pub status: TaskStatus,
    pub expected_due_at: Option<Timestamp>,
    pub completed_at: Option<Timestamp>,
    pub failure_reason: Option<String>,
    pub parent_task_id: Option<String>,
    pub schedule: Option<String>,
    pub skill_attached_event_id: Option<i64>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deliverable {
    pub id: String,
    pub task_id: String,
    pub tenant_id: Option<String>,
    pub format: String,
    pub source_content_path: Option<String>,
    pub source_content_sha256: Option<String>,
    pub rendered_content_path: String,
    pub rendered_content_sha256: String,
    pub content_size: i64,
    pub citations: Option<Vec<i64>>,
    pub provenance_manifest: serde_json::Value,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskDeliverablesResponse {
    pub deliverables: Vec<Deliverable>,
    pub latest_session_id: Option<String>,
}

// ----------------------------------------------------------------------------
// WebSocket envelopes (mirror frontend/lib/ws-types.ts ↔ server/src/ws.rs)
// ----------------------------------------------------------------------------

/// A single agent event as surfaced over the socket. `payload` carries the
/// `kind` discriminant plus the event-type-specific body (kept as raw JSON so
/// the UI can render progressively without locking the schema here).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerEvent {
    pub id: String,
    pub session_id: String,
    pub ts: Timestamp,
    pub payload: serde_json::Value,
}

impl ServerEvent {
    /// The `payload.kind` discriminant, if present.
    pub fn kind(&self) -> Option<&str> {
        self.payload.get("kind").and_then(|v| v.as_str())
    }
}

/// Inbound envelope from the server (`type` discriminated).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerEnvelope {
    Event {
        id: String,
        session_id: String,
        ts: Timestamp,
        payload: serde_json::Value,
    },
    Ack {
        id: String,
        #[serde(rename = "ref")]
        reference: String,
        ok: bool,
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
    },
    Ping {
        ts: Timestamp,
    },
    Pong {
        ts: Timestamp,
    },
    Error {
        #[serde(default)]
        id: Option<String>,
        kind: String,
        message: String,
    },
}

/// Outbound command bodies (`cmd` discriminated, nested under an envelope).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum CommandPayload {
    Subscribe {
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        from_event_id: Option<i64>,
    },
    TaskCreate {
        input: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_steps: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_cap_cents: Option<i64>,
    },
    TaskPause {
        session_id: String,
    },
    TaskResume {
        session_id: String,
    },
    TaskCancel {
        session_id: String,
    },
    UserResponse {
        session_id: String,
        in_reply_to_call_id: String,
        content: String,
    },
    BriefingConfirm {
        task_id: String,
        in_reply_to_call_id: String,
        /// "confirm" | "edit" | "cancel"
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        edits: Option<serde_json::Value>,
    },
}

/// The full client → server envelope wrapping a [`CommandPayload`].
#[derive(Debug, Clone, Serialize)]
pub struct ClientCommand {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub ts: Timestamp,
    pub payload: CommandPayload,
}

impl ClientCommand {
    /// Wrap a payload into a `type:"command"` envelope. `session_id` is lifted
    /// to the envelope when the payload carries one (mirrors lib/ws.ts).
    pub fn new(id: String, ts: Timestamp, payload: CommandPayload) -> Self {
        let session_id = match &payload {
            CommandPayload::Subscribe { session_id, .. }
            | CommandPayload::TaskPause { session_id }
            | CommandPayload::TaskResume { session_id }
            | CommandPayload::TaskCancel { session_id }
            | CommandPayload::UserResponse { session_id, .. } => Some(session_id.clone()),
            _ => None,
        };
        Self {
            kind: "command",
            id,
            session_id,
            ts,
            payload,
        }
    }
}
