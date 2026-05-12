//! Fresh-context builder for the Verifier Worker.
//!
//! Per architecture §2.4.4, every verifier run receives a *fresh*
//! prompt — never the agent's running context. This module composes
//! the 5 pieces: plan snapshot, feature-list, event window around the
//! trigger anchor, trigger description, system prompt.
//!
//! refs: /specs/phase-1/stories/story-1.9b.md
//! refs: /specs/phase-1/architecture.md §2.4.4

use serde_json::Value;

use crate::events::{Event, EventQuery, EventStore, sqlite::SqliteEventStore};
use crate::llm::types::{Message, Role};
use crate::plan::PlanManager;
use crate::sandbox::SandboxClient;

use super::{InvalidationReason, VerifyRequest, VerifyTrigger};

/// Default ± window size around the trigger anchor event id. The
/// fresh context will include up to `WINDOW_EACH_SIDE` events on each
/// side of the anchor.
pub const WINDOW_EACH_SIDE: usize = 50;

/// Hard cap on per-side window in case a deployment misconfigures it.
const WINDOW_HARD_CAP: usize = 500;

/// Build the verifier prompt's `messages` array. Returns
/// `[system_prompt, user_body]` ready to hand to the LLM client.
pub async fn build_fresh_context(
    plan_manager: &PlanManager,
    events: &SqliteEventStore,
    sandbox: &SandboxClient,
    system_prompt: &str,
    req: &VerifyRequest,
) -> Result<Vec<Message>, ContextBuildError> {
    let plan_block = match plan_manager.snapshot(&req.session_id).await {
        Ok(plan) => format!("=== PLAN ===\n{}\n", serde_json::to_string_pretty(&plan)?),
        Err(_) => String::from("=== PLAN ===\n(none yet)\n"),
    };

    // Best-effort feature-list read — silently skipped if absent.
    let feature_list_block = match sandbox
        .read_workspace_file_json::<Value>(&req.session_id, "feature-list.json")
        .await
    {
        Ok(fl) => format!(
            "\n=== FEATURE LIST ===\n{}\n",
            serde_json::to_string_pretty(&fl)?
        ),
        Err(_) => String::new(),
    };

    let window = collect_event_window(events, &req.session_id, req.triggered_at_event_id).await?;
    let mut event_block = String::from("\n=== EVENT WINDOW ===\n");
    for ev in &window {
        event_block.push_str(&format_event_for_verifier(ev));
        event_block.push('\n');
    }

    let trigger_block = format!(
        "\n=== TRIGGER ===\n{}\n",
        describe_trigger(&req.trigger, req.triggered_at_event_id)
    );

    let mut user_body = String::new();
    user_body.push_str(&trigger_block);
    user_body.push_str(&plan_block);
    user_body.push_str(&feature_list_block);
    user_body.push_str(&event_block);

    Ok(vec![
        Message {
            role: Role::System,
            content: Some(system_prompt.to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
        Message {
            role: Role::User,
            content: Some(user_body),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
    ])
}

/// One-paragraph human-readable description of why this verifier run
/// was triggered. Mirrored from architecture §2.4.4 step 4.
pub fn describe_trigger(trigger: &VerifyTrigger, anchor_event_id: u64) -> String {
    match trigger {
        VerifyTrigger::TaskComplete {
            final_message_call_id,
        } => format!(
            "TaskComplete: agent claimed task done at event {anchor_event_id} \
             via tool_call {final_message_call_id}. Independently verify the \
             claim against the recorded work."
        ),
        VerifyTrigger::Invalidation { reason } => match reason {
            InvalidationReason::FileMismatch {
                path,
                old_sha,
                new_sha,
            } => format!(
                "Invalidation: file {} changed unexpectedly (old sha {old_sha} → new sha {new_sha}). \
                 Anchor event {anchor_event_id}.",
                path.display()
            ),
        },
        VerifyTrigger::CircuitBreaker { kind } => format!(
            "CircuitBreaker[{kind:?}]: agent loop hit a safety cap at event \
             {anchor_event_id}. Decide whether the partial work is salvageable \
             via plan_update or genuinely done."
        ),
    }
}

/// Fetch a `[anchor - WINDOW_EACH_SIDE, anchor + WINDOW_EACH_SIDE]`
/// window of events for the session, in id-ascending order.
async fn collect_event_window(
    events: &SqliteEventStore,
    session_id: &str,
    anchor: u64,
) -> Result<Vec<Event>, ContextBuildError> {
    // Phase 0's EventQuery only supports `after_id`. To get ±N around
    // the anchor with one query, fetch from (anchor - N - 1) up to a
    // limit of 2N+1.
    let half = WINDOW_EACH_SIDE.min(WINDOW_HARD_CAP);
    let after_id = anchor.saturating_sub((half + 1) as u64) as i64;
    let limit = (half * 2) + 1;
    let rows = events
        .query(
            session_id,
            EventQuery {
                after_id: Some(after_id),
                event_type: None,
                limit: Some(limit),
            },
        )
        .await?;
    Ok(rows)
}

/// Single-line event rendering for the user prompt. Keep this compact
/// — the LLM needs to scan tens of events; full JSON dumps blow the
/// token budget.
pub fn format_event_for_verifier(ev: &Event) -> String {
    format!(
        "  [{}] {} src={} data={}",
        ev.id, ev.event_type, ev.source, ev.data
    )
}

#[derive(Debug, thiserror::Error)]
pub enum ContextBuildError {
    #[error("plan: {0}")]
    Plan(#[from] crate::plan::PlanError),
    #[error("events: {0}")]
    Events(#[from] crate::events::EventError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verifier::BreakerKind;

    #[test]
    fn describe_trigger_task_complete_mentions_call_id() {
        let descr = describe_trigger(
            &VerifyTrigger::TaskComplete {
                final_message_call_id: "call-xyz".into(),
            },
            42,
        );
        assert!(descr.contains("TaskComplete"));
        assert!(descr.contains("call-xyz"));
        assert!(descr.contains("42"));
    }

    #[test]
    fn describe_trigger_invalidation_mentions_path_and_shas() {
        let descr = describe_trigger(
            &VerifyTrigger::Invalidation {
                reason: InvalidationReason::FileMismatch {
                    path: std::path::PathBuf::from("/workspace/src/foo.rs"),
                    old_sha: "abc".into(),
                    new_sha: "def".into(),
                },
            },
            17,
        );
        assert!(descr.contains("Invalidation"));
        assert!(descr.contains("foo.rs"));
        assert!(descr.contains("abc"));
        assert!(descr.contains("def"));
    }

    #[test]
    fn describe_trigger_circuit_breaker_mentions_kind() {
        let descr = describe_trigger(
            &VerifyTrigger::CircuitBreaker {
                kind: BreakerKind::Stuck,
            },
            7,
        );
        assert!(descr.contains("CircuitBreaker"));
        assert!(descr.contains("Stuck"));
    }
}
