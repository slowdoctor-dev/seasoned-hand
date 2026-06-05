//! Briefing card (replaces `chat/briefing-card.tsx`). Renders an authored Brief
//! inline in the Chat panel with Confirm / Cancel actions that emit the
//! `briefing_confirm` command (keyed by task_id) over the socket.
//!
//! Foundation scope: Confirm / Cancel. The JSON-textarea **edit** flow and the
//! full resolution taxonomy (superseded / auto-confirmed) are follow-ups; this
//! tracks a local "resolved" flag via the parent.

use super::socket;
use dioxus::prelude::*;
use seasoned_hand_dto::CommandPayload;

#[component]
pub fn BriefingCard(
    brief: serde_json::Value,
    call_id: String,
    task_id: Option<String>,
    resolved: bool,
    on_resolve: EventHandler<String>,
) -> Element {
    let sock = socket();

    let goal = brief
        .get("goal")
        .and_then(|v| v.as_str())
        .unwrap_or("(no goal)")
        .to_string();
    let phase_titles: Vec<String> = brief
        .get("phases")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|p| {
                    p.get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default();
    let criteria: Vec<String> = brief
        .get("success_criteria")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let deliverables: Vec<String> = brief
        .get("expected_deliverables")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|d| {
                    let f = d.get("filename").and_then(|v| v.as_str()).unwrap_or("");
                    let fmt = d.get("format").and_then(|v| v.as_str()).unwrap_or("");
                    format!("{f} ({fmt})")
                })
                .collect()
        })
        .unwrap_or_default();

    let task_known = task_id.is_some();

    rsx! {
        div { class: "rounded border border-blue-800 bg-neutral-900 p-3",
            div { class: "mb-1 text-xs uppercase tracking-wide text-blue-400", "Briefing" }
            div { class: "mb-2 font-medium", "{goal}" }

            if !phase_titles.is_empty() {
                div { class: "mb-2",
                    div { class: "text-xs text-neutral-500", "Phases" }
                    ol { class: "ml-4 list-decimal",
                        for t in phase_titles.iter() {
                            li { "{t}" }
                        }
                    }
                }
            }
            if !criteria.is_empty() {
                div { class: "mb-2",
                    div { class: "text-xs text-neutral-500", "Success criteria" }
                    ul { class: "ml-4 list-disc",
                        for c in criteria.iter() {
                            li { "{c}" }
                        }
                    }
                }
            }
            if !deliverables.is_empty() {
                div { class: "mb-2",
                    div { class: "text-xs text-neutral-500", "Deliverables" }
                    ul { class: "ml-4 list-disc",
                        for d in deliverables.iter() {
                            li { "{d}" }
                        }
                    }
                }
            }

            if resolved {
                div { class: "text-xs text-neutral-500", "Resolved" }
            } else if !task_known {
                div { class: "text-xs text-amber-500", "Waiting for task id…" }
            } else {
                div { class: "flex gap-2",
                    button {
                        class: "rounded bg-blue-600 px-3 py-1",
                        onclick: {
                            let sock = sock.clone();
                            let call_id = call_id.clone();
                            let task_id = task_id.clone();
                            move |_| {
                                if let Some(tid) = task_id.clone() {
                                    sock.send(CommandPayload::BriefingConfirm {
                                        task_id: tid,
                                        in_reply_to_call_id: call_id.clone(),
                                        action: "confirm".to_string(),
                                        edits: None,
                                    });
                                    on_resolve.call(call_id.clone());
                                }
                            }
                        },
                        "Confirm"
                    }
                    button {
                        class: "rounded bg-neutral-700 px-3 py-1",
                        onclick: {
                            let call_id = call_id.clone();
                            let task_id = task_id.clone();
                            move |_| {
                                if let Some(tid) = task_id.clone() {
                                    sock.send(CommandPayload::BriefingConfirm {
                                        task_id: tid,
                                        in_reply_to_call_id: call_id.clone(),
                                        action: "cancel".to_string(),
                                        edits: None,
                                    });
                                    on_resolve.call(call_id.clone());
                                }
                            }
                        },
                        "Cancel"
                    }
                }
            }
        }
    }
}
