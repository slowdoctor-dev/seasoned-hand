//! Sticky context builder for the Phase 0 runner.

use serde_json::{Value, json};

use crate::agent::AgentError;
use crate::agent::narrate::NARRATE_UI_TAG;
use crate::events::{Event, EventQuery, EventStore, EventType, sqlite::SqliteEventStore};
use crate::llm::{Message, Role, ToolCall, ToolCallFunction};
use crate::plan::{PlanManager, render::sticky_render};

const SYSTEM_PROMPT: &str = "You are Seasoned Hand. Use exactly one tool call per iteration. \
Return a tool call every turn; call idle when the task is complete.";

pub(crate) async fn build_messages(
    events: &SqliteEventStore,
    plan_manager: &PlanManager,
    session_id: &str,
) -> Result<Vec<Message>, AgentError> {
    let all_events = events
        .query(
            session_id,
            EventQuery {
                limit: Some(100),
                ..Default::default()
            },
        )
        .await?;
    let mut messages = vec![Message {
        role: Role::System,
        content: Some(SYSTEM_PROMPT.into()),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }];

    if let Ok(plan) = plan_manager.snapshot(session_id).await {
        messages.push(Message {
            role: Role::System,
            content: Some(sticky_render(&plan, 1000)),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        });
    }

    if let Some(prefix) = latest_injection_prefix(&all_events) {
        messages.push(Message {
            role: Role::System,
            content: Some(prefix),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        });
    }

    // Issue #11: rebuild the tool-call protocol faithfully. Each tool call is an
    // `Action` event immediately followed by its `Observation`; emit them as a
    // paired assistant `tool_calls` message + `tool` result so no `role:"tool"`
    // message is orphaned (which providers reject with a 400). Narration is UI-only
    // (story 1.15) and injection skill events are skipped so the agent's context is
    // free of its own outward chatter. Unpaired Action/Observation events (rare — a
    // mid-dispatch failure) degrade to plain text rather than break the sequence.
    let relevant: Vec<&Event> = all_events
        .iter()
        .filter(|event| !is_injection_skill_event(event) && !is_narrate_message(event))
        .collect();
    messages.extend(pair_messages(&relevant));

    Ok(messages)
}

/// Turn the (filtered) event stream into LLM messages, pairing each tool-call
/// `Action` with its following `Observation` into a valid assistant `tool_calls`
/// then `tool` result sequence (issue #11). Pure over the event slice, so it is
/// unit tested directly.
fn pair_messages(relevant: &[&Event]) -> Vec<Message> {
    let mut messages = Vec::new();
    let mut i = 0;
    while i < relevant.len() {
        let event = relevant[i];
        match event.event_type {
            EventType::Action => {
                let call_id = string_field(event, "call_id");
                let paired_observation = call_id.as_deref().and_then(|id| {
                    relevant.get(i + 1).filter(|next| {
                        next.event_type == EventType::Observation
                            && string_field(next, "call_id").as_deref() == Some(id)
                    })
                });
                match (call_id, paired_observation) {
                    (Some(id), Some(observation)) => {
                        messages.push(assistant_tool_call(&id, event));
                        messages.push(tool_result(&id, observation));
                        i += 2;
                    }
                    _ => {
                        // Tool call with no matching observation — plain assistant
                        // text, never a dangling `tool_calls` (also a protocol error).
                        messages.push(action_as_text(event));
                        i += 1;
                    }
                }
            }
            EventType::Observation => {
                // Observation with no preceding matching Action — fold to plain text.
                messages.push(observation_as_text(event));
                i += 1;
            }
            EventType::Message => {
                messages.push(message_event(event));
                i += 1;
            }
            _ => {
                messages.push(other_event(event));
                i += 1;
            }
        }
    }
    messages
}

fn string_field(event: &Event, key: &str) -> Option<String> {
    event
        .data
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn is_narrate_message(event: &Event) -> bool {
    event.event_type == EventType::Message
        && event
            .data
            .get("ui")
            .and_then(Value::as_str)
            .is_some_and(|ui| ui == NARRATE_UI_TAG)
}

fn is_injection_skill_event(event: &Event) -> bool {
    event.event_type == EventType::Skill
        && event
            .data
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind == "injection")
}

fn latest_injection_prefix(events: &[Event]) -> Option<String> {
    events.iter().rev().find_map(|event| {
        if !is_injection_skill_event(event) {
            return None;
        }
        event
            .data
            .get("rendered_prefix")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

/// An `Action` event → the assistant turn that issued the tool call, carrying the
/// `tool_calls` array the following `tool` result must pair with (issue #11).
fn assistant_tool_call(call_id: &str, action: &Event) -> Message {
    let name = string_field(action, "tool").unwrap_or_default();
    // The Action's `body` is the (possibly file-ref) argument payload; its JSON
    // string is a valid `arguments` value — the call↔result *id* pairing is what
    // the protocol validates, not the argument bytes.
    let arguments = action
        .data
        .get("body")
        .map(|body| body.to_string())
        .unwrap_or_else(|| "{}".to_string());
    Message {
        role: Role::Assistant,
        content: None,
        name: None,
        tool_calls: Some(vec![ToolCall {
            id: call_id.to_string(),
            kind: "function".to_string(),
            function: ToolCallFunction { name, arguments },
        }]),
        tool_call_id: None,
    }
}

/// An `Observation` event → the `tool` result message paired to `call_id`.
fn tool_result(call_id: &str, observation: &Event) -> Message {
    Message {
        role: Role::Tool,
        content: Some(observation.data.to_string()),
        name: None,
        tool_calls: None,
        tool_call_id: Some(call_id.to_string()),
    }
}

/// Fallback for a tool call with no matching observation: plain assistant text
/// (never a dangling `tool_calls`, which is itself a protocol error).
fn action_as_text(action: &Event) -> Message {
    let name = string_field(action, "tool").unwrap_or_default();
    Message {
        role: Role::Assistant,
        content: Some(format!("(tool call: {name})")),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }
}

/// Fallback for an observation with no preceding action: plain user text.
fn observation_as_text(observation: &Event) -> Message {
    Message {
        role: Role::User,
        content: Some(observation.data.to_string()),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }
}

fn message_event(event: &Event) -> Message {
    let role = match event.data.get("role").and_then(Value::as_str) {
        Some("assistant") => Role::Assistant,
        _ => Role::User,
    };
    Message {
        role,
        content: event
            .data
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some(event.data.to_string())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }
}

fn other_event(event: &Event) -> Message {
    Message {
        role: Role::System,
        content: Some(
            json!({
                "event_type": event.event_type.as_str(),
                "source": event.source,
                "data": event.data,
            })
            .to_string(),
        ),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;
    use crate::events::Event;
    use serde_json::json;

    fn event(event_type: EventType, data: serde_json::Value) -> Event {
        Event {
            id: 0,
            session_id: "s1".to_string(),
            timestamp: 0,
            event_type,
            source: "test".to_string(),
            data,
        }
    }

    #[test]
    fn action_then_observation_pair_into_assistant_toolcall_and_tool_result() {
        let relevant = [
            event(
                EventType::Action,
                json!({"tool": "shell_exec", "body": {"cmd": "ls"}, "call_id": "c1"}),
            ),
            event(EventType::Observation, json!({"call_id": "c1", "ok": true})),
        ];
        let refs: Vec<&Event> = relevant.iter().collect();
        let msgs = pair_messages(&refs);
        assert_eq!(msgs.len(), 2);
        // assistant carries the tool_call with the matching id; no content.
        assert_eq!(msgs[0].role, Role::Assistant);
        let tc = msgs[0].tool_calls.as_ref().expect("tool_calls");
        assert_eq!(tc[0].id, "c1");
        assert_eq!(tc[0].function.name, "shell_exec");
        assert!(msgs[0].content.is_none());
        // tool result pairs back to the same id.
        assert_eq!(msgs[1].role, Role::Tool);
        assert_eq!(msgs[1].tool_call_id.as_deref(), Some("c1"));
    }

    #[test]
    fn unpaired_observation_does_not_emit_an_orphan_tool_message() {
        let relevant = [event(
            EventType::Observation,
            json!({"call_id": "x", "ok": true}),
        )];
        let refs: Vec<&Event> = relevant.iter().collect();
        let msgs = pair_messages(&refs);
        assert_eq!(msgs.len(), 1);
        // Folded to plain user text — never a role:"tool" without a preceding call.
        assert_eq!(msgs[0].role, Role::User);
        assert!(msgs[0].tool_call_id.is_none());
    }

    #[test]
    fn unpaired_action_does_not_emit_a_dangling_tool_call() {
        let relevant = [event(
            EventType::Action,
            json!({"tool": "browse", "body": {}, "call_id": "c2"}),
        )];
        let refs: Vec<&Event> = relevant.iter().collect();
        let msgs = pair_messages(&refs);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, Role::Assistant);
        assert!(msgs[0].tool_calls.is_none());
    }
}
