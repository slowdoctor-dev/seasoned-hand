//! Stuck-response detection for the agent runner.
//!
//! Tracks consecutive duplicate assistant outputs and escalates in two
//! stages: at 2 duplicates inject a strategy-change prompt (Phase 1
//! diversity injector picks the variant); at 4 duplicates terminate the
//! session as ERROR. The 2/4 thresholds are intentionally tight — Manus
//! validation showed agents trapped in a duplicate-response loop never
//! recover without an external nudge, so the cheaper option is to
//! interrupt early and fail loudly per PRINCIPLE #10
//! (failure-tolerant, never failure-hiding). Story 0.14 + 0.15 closed
//! Phase 0 DEBT #23 by wiring these into `agent::AgentRunner::run`.
//!
//! refs: /specs/phase-0/stories/story-0.14.md, story-0.15.md

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::llm::AssistantMessage;

pub const STUCK_WARN_AT: u32 = 2;
pub const STUCK_HARD_AT: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StuckAction {
    Continue,
    InjectStrategyPrompt { count: u32 },
    Terminate { count: u32 },
}

#[derive(Debug, Default)]
pub struct StuckTracker {
    last_hash: Option<u64>,
    duplicate_count: u32,
}

impl StuckTracker {
    pub fn observe(&mut self, message: &AssistantMessage) -> StuckAction {
        let hash = hash_message(message);
        if Some(hash) != self.last_hash {
            self.last_hash = Some(hash);
            self.duplicate_count = 0;
            return StuckAction::Continue;
        }

        self.duplicate_count += 1;
        let total_repeated = self.duplicate_count + 1;
        if total_repeated >= STUCK_HARD_AT {
            StuckAction::Terminate {
                count: total_repeated,
            }
        } else if total_repeated >= STUCK_WARN_AT {
            StuckAction::InjectStrategyPrompt {
                count: total_repeated,
            }
        } else {
            StuckAction::Continue
        }
    }
}

fn hash_message(message: &AssistantMessage) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{:?}", message.role).hash(&mut hasher);
    normalize_whitespace(message.content.as_deref().unwrap_or("")).hash(&mut hasher);

    let mut tool_calls_signature = message
        .tool_calls
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|call| (call.function.name.clone(), call.function.arguments.clone()))
        .collect::<Vec<_>>();
    tool_calls_signature.sort();
    tool_calls_signature.hash(&mut hasher);
    hasher.finish()
}

fn normalize_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{Role, ToolCall, ToolCallFunction};

    fn msg(content: &str, calls: Vec<(&str, &str)>) -> AssistantMessage {
        AssistantMessage {
            role: Role::Assistant,
            content: Some(content.into()),
            tool_calls: Some(
                calls
                    .into_iter()
                    .map(|(name, args)| ToolCall {
                        id: name.into(),
                        kind: "function".into(),
                        function: ToolCallFunction {
                            name: name.into(),
                            arguments: args.into(),
                        },
                    })
                    .collect(),
            ),
        }
    }

    #[test]
    fn single_unique_response_returns_continue() {
        let mut tracker = StuckTracker::default();
        assert_eq!(
            tracker.observe(&msg("one", vec![("idle", "{}")])),
            StuckAction::Continue
        );
    }

    #[test]
    fn two_duplicates_returns_inject() {
        let mut tracker = StuckTracker::default();
        let message = msg("same", vec![("idle", "{}")]);
        assert_eq!(tracker.observe(&message), StuckAction::Continue);
        assert_eq!(
            tracker.observe(&message),
            StuckAction::InjectStrategyPrompt { count: 2 }
        );
    }

    #[test]
    fn three_duplicates_still_inject_with_higher_count() {
        let mut tracker = StuckTracker::default();
        let message = msg("same", vec![("idle", "{}")]);
        assert_eq!(tracker.observe(&message), StuckAction::Continue);
        assert_eq!(
            tracker.observe(&message),
            StuckAction::InjectStrategyPrompt { count: 2 }
        );
        assert_eq!(
            tracker.observe(&message),
            StuckAction::InjectStrategyPrompt { count: 3 }
        );
    }

    #[test]
    fn four_duplicates_returns_terminate() {
        let mut tracker = StuckTracker::default();
        let message = msg("same", vec![("idle", "{}")]);
        assert_eq!(tracker.observe(&message), StuckAction::Continue);
        assert_eq!(
            tracker.observe(&message),
            StuckAction::InjectStrategyPrompt { count: 2 }
        );
        assert_eq!(
            tracker.observe(&message),
            StuckAction::InjectStrategyPrompt { count: 3 }
        );
        assert_eq!(
            tracker.observe(&message),
            StuckAction::Terminate { count: 4 }
        );
    }

    #[test]
    fn alternating_responses_keep_counter_at_zero() {
        let mut tracker = StuckTracker::default();
        assert_eq!(
            tracker.observe(&msg("one", vec![("idle", "{}")])),
            StuckAction::Continue
        );
        assert_eq!(
            tracker.observe(&msg("two", vec![("idle", "{}")])),
            StuckAction::Continue
        );
        assert_eq!(
            tracker.observe(&msg("one", vec![("idle", "{}")])),
            StuckAction::Continue
        );
    }

    #[test]
    fn whitespace_differences_count_as_duplicate() {
        let mut tracker = StuckTracker::default();
        assert_eq!(
            tracker.observe(&msg("same words", vec![("idle", "{}")])),
            StuckAction::Continue
        );
        assert_eq!(
            tracker.observe(&msg(" same\n\nwords ", vec![("idle", "{}")])),
            StuckAction::InjectStrategyPrompt { count: 2 }
        );
    }

    #[test]
    fn tool_args_differ_count_as_unique() {
        let mut tracker = StuckTracker::default();
        assert_eq!(
            tracker.observe(&msg(
                "",
                vec![("message_notify_user", r#"{"content":"a"}"#)]
            )),
            StuckAction::Continue
        );
        assert_eq!(
            tracker.observe(&msg(
                "",
                vec![("message_notify_user", r#"{"content":"b"}"#)]
            )),
            StuckAction::Continue
        );
    }
}
