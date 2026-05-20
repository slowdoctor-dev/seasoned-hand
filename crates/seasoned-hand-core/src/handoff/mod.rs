//! Task hand-off lifecycle (story 5.9 / architecture §5).
//!
//! Moving task ownership from one user to another is the headline
//! delegation primitive of Phase 5. The state machine + audit_log write
//! live here; HTTP and CLI surfaces dispatch through this module so
//! every reassignment path satisfies the same invariants:
//!
//! 1. **State gate**: Running tasks must pause before reassignment.
//!    Completed/Failed/Cancelled tasks reject reassignment outright.
//! 2. **Atomicity**: owner update, `task_paused_for_handoff` event,
//!    and `audit_log` row land in one DB transaction so partial
//!    state is never externally visible.
//! 3. **Authorization**: only roles whose §4.3 row admits `TaskHandoff`
//!    can call this — the policy engine checks the actor's effective
//!    role + the resource's `is_same_org` flag before any write.
//! 4. **Optimistic concurrency**: callers pass an `expected_updated_at`;
//!    if the row has moved since the caller last read it, the transition
//!    rejects with [`HandoffError::StaleRevision`] (story 5.21 wires this
//!    through HTTP as a 409).
//!
//! refs: /specs/phase-5/stories/story-5.9.md

pub mod task;
pub use task::{HandoffError, HandoffOutcome, HandoffRequest, TaskHandoffService};

#[cfg(test)]
mod tests;
