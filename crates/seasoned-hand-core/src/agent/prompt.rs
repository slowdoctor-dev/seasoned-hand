//! Sticky context builder for the Phase 0 runner.

use serde_json::{Value, json};

use crate::agent::AgentError;
use crate::agent::narrate::NARRATE_UI_TAG;
use crate::events::{Event, EventQuery, EventStore, EventType, sqlite::SqliteEventStore};
use crate::llm::{Message, Role};
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

    for event in all_events {
        if is_narrate_message(&event) {
            // Story 1.15: narration is UI-only signal; skipping it
            // here keeps the agent's own context free of its own
            // outward chatter (architecture §12 q2).
            continue;
        }
        messages.push(event_to_message(&event));
    }

    Ok(messages)
}

fn is_narrate_message(event: &Event) -> bool {
    event.event_type == EventType::Message
        && event
            .data
            .get("ui")
            .and_then(Value::as_str)
            .is_some_and(|ui| ui == NARRATE_UI_TAG)
}

fn event_to_message(event: &Event) -> Message {
    match event.event_type {
        EventType::Message => {
            let role = match event.data.get("role").and_then(|value| value.as_str()) {
                Some("assistant") => Role::Assistant,
                _ => Role::User,
            };
            Message {
                role,
                content: event
                    .data
                    .get("content")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .or_else(|| Some(event.data.to_string())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            }
        }
        EventType::Observation => Message {
            role: Role::Tool,
            content: Some(event.data.to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: event
                .data
                .get("call_id")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        },
        _ => Message {
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
        },
    }
}
