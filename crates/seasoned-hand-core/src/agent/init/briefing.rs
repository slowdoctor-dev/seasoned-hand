//! Briefing protocol — confirm gate types + helpers for
//! [`super::Initializer::run_with_confirmation`].
//!
//! Story 2.8 lands the `UserResponse` shape that channel handlers
//! forward into the per-task mpsc receiver, the `RunConfig` knobs the
//! Initializer reads, and the `RunOutcome` returned to the caller. The
//! emit-and-wait loop itself lives in `super` so it can reach the
//! private helpers (`emit_*`, `seed_plan`) without re-exposing them.
//!
//! Wiring the per-task mpsc through the `IntakeRouter` spawn path +
//! `WS task_create` handler is **deferred** — see Phase 2 DEBT #13 +
//! #15. This module only provides the Initializer side of the
//! protocol; the AppState sender-map + WS `briefing_confirm` cmd
//! handler land in a follow-up.
//!
//! refs: /specs/phase-2/architecture.md §2.2, §8 "Briefing confirmation timeout"
//! refs: /specs/phase-2/stories/story-2.8.md

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::project::brief::{Brief, BriefPhase, DeliverableSpec};

/// Maximum number of `edit` cycles before the Initializer rejects the
/// next edit with [`BriefingError::TooManyEdits`]. Surfaces in the UI
/// as "please cancel and restart with a clearer brief" and prevents an
/// infinite edit loop pinning a task in the gate forever.
pub const MAX_EDIT_CYCLES: u32 = 5;

/// Default auto-confirm window (architecture §2.2 / §8). 5 minutes.
pub const DEFAULT_CONFIRM_TIMEOUT_SECS: u64 = 300;

/// Channel-agnostic envelope the WS / webhook / email `briefing_confirm`
/// handlers translate into. The Initializer consumes these off a
/// per-task mpsc — the sender is owned by the IntakeRouter spawn path
/// (DEBT #13) so any registered channel can route a user's reply
/// uniformly.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserResponse {
    pub in_reply_to_call_id: String,
    pub action: BriefingAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BriefingAction {
    Confirm,
    Edit { edits: PartialBrief },
    Cancel,
}

/// Subset of [`Brief`] carried by an `edit` action. Each field is
/// optional — present values overwrite the current brief, absent
/// values leave the existing field untouched.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PartialBrief {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phases: Option<Vec<BriefPhase>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_criteria: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_deliverables: Option<Vec<DeliverableSpec>>,
}

#[derive(Clone, Debug)]
pub struct RunConfig {
    /// Wall-clock window before auto-confirm fires. Ignored when
    /// `require_confirm` is true.
    pub confirm_timeout: Duration,
    /// `briefing_require_confirm: true` operator policy — disables the
    /// auto-confirm timer entirely; the wait is unbounded.
    pub require_confirm: bool,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            confirm_timeout: Duration::from_secs(DEFAULT_CONFIRM_TIMEOUT_SECS),
            require_confirm: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    /// Confirmed (manually or auto). Plan seeded, task moved to
    /// `running` — Worker loop is ready to run.
    Started,
    /// User cancelled. Task moved to `cancelled`; Plan not seeded.
    Cancelled,
}

/// Apply a [`PartialBrief`] on top of a [`Brief`] in-place. Caller
/// re-runs [`Brief::validate`] after this returns.
pub(super) fn apply_edits(brief: &mut Brief, edits: PartialBrief) {
    if let Some(goal) = edits.goal {
        brief.goal = goal;
    }
    if let Some(phases) = edits.phases {
        brief.phases = phases;
    }
    if let Some(sc) = edits.success_criteria {
        brief.success_criteria = sc;
    }
    if let Some(eds) = edits.expected_deliverables {
        brief.expected_deliverables = eds;
    }
}
