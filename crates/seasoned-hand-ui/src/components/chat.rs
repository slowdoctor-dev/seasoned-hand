//! Chat / delegation panel (replaces `chat.tsx`). Renders the live event stream
//! for the active session and sends task_create / user_response commands.
//!
//! Foundation scope: renders events + sends commands. The briefing-card flow
//! (`briefing-card.tsx`) and ack-driven session capture are Phase 6 follow-ups
//! (see ws.rs note on ack handling).

use super::{selection, socket};
use crate::dto::{CommandPayload, ServerEvent};
use dioxus::prelude::*;

#[component]
pub fn Chat() -> Element {
    let sel = selection();
    let sock = socket();
    let session = sel.session_id;
    let events = sock.events;
    let mut input = use_signal(String::new);

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
                                ChatEvent { key: "{e.id}", ev: e.clone() }
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
