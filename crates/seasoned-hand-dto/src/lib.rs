//! Seasoned Hand — shared wire DTOs (ADR-016, Phase 6 story 6.3).
//!
//! Single source of truth for the data shapes that cross the `/v1` REST +
//! `/ws` WebSocket boundary. Depended on by `seasoned-hand-core` (which
//! re-exports the domain entities) and `seasoned-hand-ui` (which consumes them
//! directly), eliminating the hand-mirrored duplication that existed while the
//! UI was a separate TypeScript app (removed in the ADR-016 cutover, #5).
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
// Verifier DTOs (GET /v1/sessions/:id/verifications)
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verification {
    pub id: String,
    pub session_id: String,
    pub triggered_at_event_id: i64,
    pub trigger_kind: String,
    #[serde(default)]
    pub trigger_detail: serde_json::Value,
    pub verdict: Verdict,
    pub reason: String,
    pub evidence_event_ids: Vec<i64>,
    #[serde(default)]
    pub suggested_plan_update: Option<serde_json::Value>,
    pub model_id: String,
    pub cost_cents: i64,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationListResponse {
    pub rows: Vec<Verification>,
    pub next_cursor: Option<i64>,
}

// ----------------------------------------------------------------------------
// Workspace file listing (GET /v1/workspace/:session_id/)
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub name: String,
    /// "file" | "dir"
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum WorkspaceListing {
    Dir { entries: Vec<WorkspaceEntry> },
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
        // `default` for deserialize tolerance; `skip_serializing_if` so the
        // serialized form matches the server's (omit when None) — story 6.3c.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    Ping {
        ts: Timestamp,
    },
    Pong {
        ts: Timestamp,
    },
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        kind: String,
        message: String,
    },
}

/// Client→server command — the **single** source of truth shared by the Dioxus UI
/// (which serializes it) and the server (which deserializes it), issue #19. JSON is
/// tagged on `cmd`. Previously this was duplicated as a server-local copy that had
/// drifted (durable pause, `u32` widths, the typed `action`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        max_steps: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_cap_cents: Option<u32>,
    },
    /// Story 2.16: `durable` is additive. `Some(true)` / `None` (default) → durable
    /// pause; `Some(false)` → plain pause.
    TaskPause {
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        durable: Option<bool>,
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
    /// `edits` carries a `PartialBrief` as raw JSON (only consulted when
    /// `action == Edit`); kept as `Value` so this wasm-safe crate does not depend
    /// on the native `seasoned-hand-core`.
    BriefingConfirm {
        task_id: String,
        in_reply_to_call_id: String,
        action: BriefingActionTag,
        #[serde(skip_serializing_if = "Option::is_none")]
        edits: Option<serde_json::Value>,
    },
}

/// Briefing-confirm verb (issue #19; moved here from the server). Serializes
/// snake_case: `confirm` / `edit` / `cancel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BriefingActionTag {
    Confirm,
    Edit,
    Cancel,
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
            | CommandPayload::TaskPause { session_id, .. }
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

#[cfg(test)]
mod command_payload_tests {
    use super::*;

    fn roundtrip(payload: CommandPayload) {
        // Issue #19: one shared type — what the UI serializes, the server
        // deserializes — so serialize↔deserialize must be symmetric.
        let json = serde_json::to_string(&payload).unwrap();
        let back: CommandPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(payload, back);
    }

    #[test]
    fn durable_pause_roundtrips_and_serializes_the_field() {
        let pause = CommandPayload::TaskPause {
            session_id: "s1".into(),
            durable: Some(true),
        };
        let json = serde_json::to_string(&pause).unwrap();
        assert!(json.contains("\"cmd\":\"task_pause\""));
        assert!(json.contains("\"durable\":true"));
        roundtrip(pause);
        // Omitted durable still deserializes (defaults to None → durable pause).
        let parsed: CommandPayload =
            serde_json::from_str(r#"{"cmd":"task_pause","session_id":"s1"}"#).unwrap();
        assert_eq!(
            parsed,
            CommandPayload::TaskPause {
                session_id: "s1".into(),
                durable: None
            }
        );
    }

    #[test]
    fn task_create_uses_u32_widths() {
        roundtrip(CommandPayload::TaskCreate {
            input: "do it".into(),
            max_steps: Some(24),
            cost_cap_cents: Some(500),
        });
    }

    #[test]
    fn briefing_confirm_action_is_snake_case_enum() {
        let confirm = CommandPayload::BriefingConfirm {
            task_id: "t1".into(),
            in_reply_to_call_id: "c1".into(),
            action: BriefingActionTag::Edit,
            edits: Some(serde_json::json!({"goal": "x"})),
        };
        let json = serde_json::to_string(&confirm).unwrap();
        assert!(json.contains("\"action\":\"edit\""));
        roundtrip(confirm);
    }
}
