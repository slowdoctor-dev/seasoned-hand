//! Verifier (L4 meta-cognition) — DB layer + types only in story 1.9.
//!
//! The worker loop, Redis Streams consumer, fresh-context builder,
//! verdict parser, and watchdog all land in story 1.9b. Types are
//! declared here so stories 1.10 (TaskComplete trigger), 1.11
//! (Invalidation), and 1.12 (CircuitBreaker) can construct
//! [`VerifyRequest`] values before the worker exists.
//!
//! refs: /specs/phase-1/stories/story-1.9.md
//! refs: /specs/phase-1/architecture.md §2.4 (Verifier), §3.1 (table),
//!       §3.2 (state widening), §2.4.3 (FAIL-biased prompt)

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod context;
pub mod parse;
pub mod persistence;
pub mod routes;
pub mod worker;

pub use persistence::{VerificationStore, VerifierPersistenceError};
pub use worker::{Worker, WorkerDeps, handle_request_with_watchdog};

/// Trigger source for a single verifier run. Stored as `trigger_kind`
/// (one of `TaskComplete`/`Invalidation`/`CircuitBreaker`) plus the
/// fully-serialised payload in `trigger_detail` (JSON text column).
///
/// JSON shape uses an internal `trigger` discriminator tag (not `kind`,
/// because the `CircuitBreaker` variant carries an inner `kind` field).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "trigger")]
pub enum VerifyTrigger {
    TaskComplete { final_message_call_id: String },
    Invalidation { reason: InvalidationReason },
    CircuitBreaker { kind: BreakerKind },
}

impl VerifyTrigger {
    /// String tag stored in the `trigger_kind` column.
    pub fn kind_str(&self) -> &'static str {
        match self {
            VerifyTrigger::TaskComplete { .. } => "TaskComplete",
            VerifyTrigger::Invalidation { .. } => "Invalidation",
            VerifyTrigger::CircuitBreaker { .. } => "CircuitBreaker",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "reason")]
pub enum InvalidationReason {
    FileMismatch {
        path: PathBuf,
        old_sha: String,
        new_sha: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BreakerKind {
    Stuck,
    Cost,
    MaxSteps,
    ErrorRate,
}

/// Input handed to the (story 1.9b) Verifier Worker via Redis Streams.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyRequest {
    pub session_id: String,
    pub trigger: VerifyTrigger,
    pub triggered_at_event_id: u64,
    #[serde(default)]
    pub context_hint: VerifyContextHint,
}

/// Reserved for story 1.9b's fresh-context builder. Empty in 1.9.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerifyContextHint;

/// Verifier output as persisted in the `verifications` table. The
/// runtime parses model output into this; story 1.9 only declares the
/// shape and round-trips it through DB.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VerdictKind {
    Pass,
    Fail,
}

impl VerdictKind {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            VerdictKind::Pass => "pass",
            VerdictKind::Fail => "fail",
        }
    }
}

/// One row of the `verifications` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verification {
    pub id: String,
    pub session_id: String,
    pub triggered_at_event_id: i64,
    pub trigger_kind: String,
    /// JSON-encoded `VerifyTrigger` payload (the full enum variant
    /// including its fields), stored as TEXT.
    pub trigger_detail: serde_json::Value,
    pub verdict: VerdictKind,
    pub reason: String,
    pub evidence_event_ids: Vec<i64>,
    /// `None` if the verifier did not propose a plan revision; otherwise
    /// arbitrary JSON shaped like `{ "phases": [...] }`.
    pub suggested_plan_update: Option<serde_json::Value>,
    pub model_id: String,
    pub cost_cents: i64,
    pub created_at: i64,
}

/// Insertion payload — the runtime supplies what the LLM returned plus
/// attribution metadata; `id` and `created_at` are filled in by
/// [`persistence::VerificationStore::insert`].
#[derive(Debug, Clone)]
pub struct NewVerification {
    pub session_id: String,
    pub triggered_at_event_id: i64,
    pub trigger: VerifyTrigger,
    pub verdict: VerdictKind,
    pub reason: String,
    pub evidence_event_ids: Vec<i64>,
    pub suggested_plan_update: Option<serde_json::Value>,
    pub model_id: String,
    pub cost_cents: i64,
}

#[derive(Debug, Error)]
pub enum VerifierError {
    #[error("persistence: {0}")]
    Persistence(#[from] VerifierPersistenceError),
    #[error("verifier system prompt template missing at {path}: {source}")]
    PromptMissing {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Read the verifier FAIL-biased system prompt from
/// `config/prompts/verifier.system.txt`. Server boot calls this only
/// when `verifier_enabled` is true; missing file is a startup-fatal
/// configuration error per acceptance criteria.
pub fn load_system_prompt(path: &str) -> Result<String, VerifierError> {
    std::fs::read_to_string(path).map_err(|source| VerifierError::PromptMissing {
        path: path.to_string(),
        source,
    })
}

#[cfg(test)]
mod tests;
