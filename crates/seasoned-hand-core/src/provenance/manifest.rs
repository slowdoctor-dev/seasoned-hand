//! Provenance manifest schema (architecture §2.11).
//!
//! Every Deliverable carries a manifest that traces it back to evidence:
//! the originating intake event, the briefing, every session that ran,
//! decision events, verifier verdicts, checkpoints, aggregate metrics,
//! and the delivery events that handed it back to the operator.
//!
//! refs: /specs/phase-2/architecture.md §2.11

use serde::{Deserialize, Serialize};

/// Hard-coded schema version. Bumping requires a new manifest reader
/// that knows how to interpret older payloads (Phase 3+).
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceManifest {
    pub schema_version: u32,
    pub task_id: String,
    pub project_id: String,
    pub tenant_id: Option<String>,
    pub intake: IntakeProvenance,
    pub brief: BriefProvenance,
    pub sessions: Vec<SessionProvenance>,
    pub decisions: Vec<i64>,
    pub verifier_verdicts: Vec<String>,
    pub checkpoints: Vec<CheckpointProvenance>,
    pub metrics: ProvenanceMetrics,
    pub delivered_to: Vec<DeliveredTo>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source_content_sha256: Option<String>,
    pub rendered_content_sha256: String,
    pub citations: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntakeProvenance {
    pub channel: String,
    pub intake_id: String,
    pub received_at: i64,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefProvenance {
    /// Event id of the canonical `Misc{kind:"briefing"}` for the task.
    /// `None` when no briefing event was emitted (legacy / direct-create
    /// tasks).
    pub brief_event_id: Option<i64>,
    pub confirmed: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub confirmed_at: Option<i64>,
    pub edits_applied: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionProvenance {
    pub id: String,
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ended_at: Option<i64>,
    /// `"completed" | "paused" | "cancelled" | "failed"` (§2.11). `None`
    /// when the session is still live (state ∈ IDLE/RUNNING/VERIFYING).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub end_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointProvenance {
    pub checkpoint_id: String,
    pub git_sha: String,
    pub rolled_back: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceMetrics {
    pub tool_calls: u64,
    pub cost_cents: i64,
    pub wall_seconds: i64,
    pub sessions_count: u32,
    pub pause_resume_cycles: u32,
    pub verifier_runs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveredTo {
    pub channel: String,
    pub delivery_id: String,
    pub delivered_at: i64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub external_id: Option<String>,
}
