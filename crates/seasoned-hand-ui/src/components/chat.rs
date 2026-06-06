//! Chat / delegation panel (replaces `chat.tsx`). Renders the live event stream
//! for the active session and sends task_create / user_response commands.
//!
//! Renders events + sends commands, swaps briefing events for an interactive
//! [`BriefingCard`], and relies on the ws.rs ack handler to capture the
//! task_create session_id. Remaining follow-ups: the briefing JSON-edit flow
//! and surfacing server `error` envelopes.

use super::briefing_card::BriefingCard;
use super::{selection, socket};
use dioxus::prelude::*;
use seasoned_hand_dto::{CommandPayload, ServerEvent};
use std::collections::HashSet;

/// Peek at a ServerEvent and, if it is a `Misc{kind_tag:"briefing"}`, return the
/// `(call_id, task_id, brief)` so the chat scroller renders a [`BriefingCard`].
/// Wire shape: `{kind:"Misc", kind_tag:"briefing", data:{briefing_call_id,
/// task_id, brief}}`.
fn extract_briefing(ev: &ServerEvent) -> Option<(String, Option<String>, serde_json::Value)> {
    let p = &ev.payload;
    if p.get("kind").and_then(|v| v.as_str()) != Some("Misc") {
        return None;
    }
    if p.get("kind_tag").and_then(|v| v.as_str()) != Some("briefing") {
        return None;
    }
    let data = p.get("data")?;
    let call_id = data
        .get("briefing_call_id")
        .and_then(|v| v.as_str())?
        .to_string();
    let task_id = data
        .get("task_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let brief = data
        .get("brief")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    Some((call_id, task_id, brief))
}

#[component]
pub fn Chat() -> Element {
    let sel = selection();
    let sock = socket();
    let session = sel.session_id;
    let events = sock.events;
    let mut input = use_signal(String::new);
    // Briefing call_ids the user has locally confirmed/cancelled this session.
    let mut resolved = use_signal(HashSet::<String>::new);

    // Subscribe to the session's event stream whenever the active session
    // changes (replay from the beginning).
    {
        let sock = sock.clone();
        use_effect(move || {
            if let Some(sid) = session() {
                sock.send(CommandPayload::Subscribe {
                    session_id: sid,
                    from_event_id: Some(0),
                });
            }
        });
    }

    let send_sock = sock.clone();
    let on_submit = move |evt: FormEvent| {
        evt.prevent_default();
        let text = input.peek().trim().to_string();
        if text.is_empty() {
            return;
        }
        match session() {
            Some(sid) => send_sock.send(CommandPayload::UserResponse {
                session_id: sid,
                in_reply_to_call_id: String::new(),
                content: text,
            }),
            None => send_sock.send(CommandPayload::TaskCreate {
                input: text,
                max_steps: None,
                cost_cap_cents: None,
            }),
        }
        input.set(String::new());
    };

    rsx! {
        div { class: "flex h-full flex-col",
            div { class: "flex-1 space-y-2 overflow-y-auto p-3",
                {
                    let sid = session();
                    let evs = events();
                    let filtered: Vec<ServerEvent> = evs
                        .iter()
                        .filter(|e| sid.as_ref() == Some(&e.session_id))
                        .cloned()
                        .collect();
                    if filtered.is_empty() {
                        rsx! { div { class: "text-neutral-600", "No activity yet. Delegate a task below." } }
                    } else {
                        rsx! {
                            for e in filtered {
                                if let Some((call_id, task_id, brief)) = extract_briefing(&e) {
                                    BriefingCard {
                                        key: "{e.id}",
                                        brief,
                                        call_id: call_id.clone(),
                                        task_id,
                                        resolved: resolved().contains(&call_id),
                                        on_resolve: move |cid: String| {
                                            resolved.write().insert(cid);
                                        },
                                    }
                                } else {
                                    ChatEvent { key: "{e.id}", ev: e.clone() }
                                }
                            }
                        }
                    }
                }
            }
            form { class: "flex gap-2 border-t border-neutral-800 p-2", onsubmit: on_submit,
                input {
                    class: "flex-1 rounded bg-neutral-900 px-2 py-1 outline-none",
                    value: "{input}",
                    placeholder: "Delegate a task…",
                    oninput: move |e| input.set(e.value()),
                }
                button { r#type: "submit", class: "rounded bg-blue-600 px-3 py-1", "Send" }
            }
        }
    }
}

#[component]
fn ChatEvent(ev: ServerEvent) -> Element {
    let kind = ev.kind().unwrap_or("Event").to_string();
    let body = ev
        .payload
        .get("content")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| ev.payload.to_string());
    rsx! {
        div { class: "rounded bg-neutral-900 px-2 py-1",
            span { class: "mr-2 text-xs uppercase text-neutral-500", "{kind}" }
            span { class: "whitespace-pre-wrap break-words", "{body}" }
        }
    }
}
