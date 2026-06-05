//! Seasoned Hand — shared wire DTOs (ADR-016, Phase 6 story 6.3).
//!
//! Single source of truth for the data shapes that cross the `/v1` REST +
//! `/ws` WebSocket boundary. Depended on by `seasoned-hand-core` (which
//! re-exports the domain entities) and `seasoned-hand-ui` (which consumes them
//! directly), eliminating the hand-mirrored duplication that existed between
//! the Rust backend and the TypeScript frontend.
//!
//! Constraint: this crate is wasm-safe — pure serde, no I/O dependencies — so a
//! single definition serves both the native control plane and the wasm UI.

use serde::{Deserialize, Serialize};

/// Backend timestamps are integer epoch values (micros).
pub type Timestamp = i64;

/// Returned when a DB / wire string does not map to a known enum variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumParseError {
    pub kind: &'static str,
    pub value: String,
}

impl std::fmt::Display for EnumParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown {} variant: {}", self.kind, self.value)
    }
}

impl std::error::Error for EnumParseError {}

// ----------------------------------------------------------------------------
// Domain entities (canonical home; re-exported by seasoned-hand-core)
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStatus {
    Active,
    Archived,
}

impl ProjectStatus {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            ProjectStatus::Active => "active",
            ProjectStatus::Archived => "archived",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, EnumParseError> {
        match s {
            "active" => Ok(ProjectStatus::Active),
            "archived" => Ok(ProjectStatus::Archived),
            other => Err(EnumParseError {
                kind: "project status",
                value: other.to_string(),
            }),
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

impl TaskStatus {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            TaskStatus::Drafted => "drafted",
            TaskStatus::Briefed => "briefed",
            TaskStatus::Confirmed => "confirmed",
            TaskStatus::Running => "running",
            TaskStatus::Paused => "paused",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, EnumParseError> {
        match s {
            "drafted" => Ok(TaskStatus::Drafted),
            "briefed" => Ok(TaskStatus::Briefed),
            "confirmed" => Ok(TaskStatus::Confirmed),
            "running" => Ok(TaskStatus::Running),
            "paused" => Ok(TaskStatus::Paused),
            "completed" => Ok(TaskStatus::Completed),
            "failed" => Ok(TaskStatus::Failed),
            "cancelled" => Ok(TaskStatus::Cancelled),
            other => Err(EnumParseError {
                kind: "task status",
                value: other.to_string(),
            }),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        )
    }
}

/// Single inspectable table of legal `from → to` task transitions.
pub fn legal_transitions(from: TaskStatus) -> &'static [TaskStatus] {
    use TaskStatus::*;
    match from {
        Drafted => &[Briefed, Cancelled],
        Briefed => &[Confirmed, Cancelled],
        Confirmed => &[Running, Cancelled],
        Running => &[Paused, Completed, Failed, Cancelled],
        Paused => &[Running, Completed, Failed, Cancelled],
        Completed | Failed | Cancelled => &[],
    }
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
// Session DTOs (frontend-facing; server adoption is story 6.3b)
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SessionState {
    Idle,
    Running,
    Finished,
    Error,
    Suspended,
}

impl SessionState {
    /// Canonical DB / wire string (matches the `#[serde]` UPPERCASE rep and the
    /// values the control plane writes to the `sessions.state` column).
    pub fn as_db_str(&self) -> &'static str {
        match self {
            SessionState::Idle => "IDLE",
            SessionState::Running => "RUNNING",
            SessionState::Finished => "FINISHED",
            SessionState::Error => "ERROR",
            SessionState::Suspended => "SUSPENDED",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, EnumParseError> {
        match s {
            "IDLE" => Ok(SessionState::Idle),
            "RUNNING" => Ok(SessionState::Running),
            "FINISHED" => Ok(SessionState::Finished),
            "ERROR" => Ok(SessionState::Error),
            "SUSPENDED" => Ok(SessionState::Suspended),
            other => Err(EnumParseError {
                kind: "session state",
                value: other.to_string(),
            }),
        }
    }
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
pub struct Sandbox {
    pub novnc_url: String,
    pub ttyd_url: String,
    pub api_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDetail {
    #[serde(flatten)]
    pub summary: SessionSummary,
    pub sandbox: Option<Sandbox>,
}

// ----------------------------------------------------------------------------
// WebSocket envelopes (mirror server/src/ws.rs; server adoption is story 6.3b)
// ----------------------------------------------------------------------------

/// A single agent event surfaced over the socket. `payload` carries the `kind`
/// discriminant plus the event-type-specific body as raw JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerEvent {
    pub id: String,
    pub session_id: String,
    pub ts: Timestamp,
    pub payload: serde_json::Value,
}

impl ServerEvent {
    pub fn kind(&self) -> Option<&str> {
        self.payload.get("kind").and_then(|v| v.as_str())
    }
}

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
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        edits: Option<serde_json::Value>,
    },
}

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
