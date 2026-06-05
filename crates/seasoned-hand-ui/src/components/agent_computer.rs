//! AgentComputer panel (replaces `agent-computer.tsx` + its tab set). Hosts the
//! three JS-interop surfaces (browser/terminal/editor) plus a raw event log.
//!
//! Foundation scope: tab shell + interop mounts wired to the active session's
//! sandbox URLs. The richer tabs (deliverables, verifier, decisions, file-tree,
//! screenshot strip) from the React app are Phase 6 follow-ups.

use super::{selection, socket};
use crate::api;
use crate::interop::{MonacoEditor, NoVnc, XtermTerminal};
use dioxus::prelude::*;
use seasoned_hand_dto::ServerEvent;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Browser,
    Terminal,
    Editor,
    Files,
    Deliverables,
    Verifier,
    Decisions,
    Events,
}

#[component]
pub fn AgentComputer() -> Element {
    let sel = selection();
    let sock = socket();
    let session = sel.session_id;
    let events = sock.events;
    let mut tab = use_signal(|| Tab::Browser);

    let detail = use_resource(move || async move {
        match session() {
            Some(sid) => api::get_session(&sid).await.ok(),
            None => None,
        }
    });
    let sandbox = match &*detail.read_unchecked() {
        Some(Some(d)) => d.sandbox.clone(),
        _ => None,
    };

    let tab_cls = move |t: Tab| {
        if tab() == t {
            "px-3 py-1.5 border-b-2 border-blue-500 text-neutral-100"
        } else {
            "px-3 py-1.5 border-b-2 border-transparent text-neutral-400 hover:text-neutral-200"
        }
    };

    rsx! {
        div { class: "flex h-full flex-col bg-neutral-925",
            nav { class: "flex border-b border-neutral-800 text-xs",
                button { class: tab_cls(Tab::Browser), onclick: move |_| tab.set(Tab::Browser), "Browser" }
                button { class: tab_cls(Tab::Terminal), onclick: move |_| tab.set(Tab::Terminal), "Terminal" }
                button { class: tab_cls(Tab::Editor), onclick: move |_| tab.set(Tab::Editor), "Editor" }
                button { class: tab_cls(Tab::Files), onclick: move |_| tab.set(Tab::Files), "Files" }
                button { class: tab_cls(Tab::Deliverables), onclick: move |_| tab.set(Tab::Deliverables), "Deliverables" }
                button { class: tab_cls(Tab::Verifier), onclick: move |_| tab.set(Tab::Verifier), "Verifier" }
                button { class: tab_cls(Tab::Decisions), onclick: move |_| tab.set(Tab::Decisions), "Decisions" }
                button { class: tab_cls(Tab::Events), onclick: move |_| tab.set(Tab::Events), "Events" }
            }
            div { class: "min-h-0 flex-1 overflow-hidden",
                match tab() {
                    Tab::Browser => match &sandbox {
                        Some(s) => rsx! { NoVnc { novnc_url: s.novnc_url.clone() } },
                        None => rsx! { Placeholder { label: "No sandbox / browser for this session" } },
                    },
                    Tab::Terminal => match &sandbox {
                        Some(s) => rsx! { XtermTerminal { ws_url: s.ttyd_url.clone() } },
                        None => rsx! { Placeholder { label: "No terminal for this session" } },
                    },
                    Tab::Editor => rsx! {
                        MonacoEditor { value: "// select a file".to_string(), language: "rust".to_string() }
                    },
                    Tab::Files => rsx! { FilesTab {} },
                    Tab::Deliverables => rsx! { DeliverablesTab {} },
                    Tab::Verifier => rsx! { VerifierTab {} },
                    Tab::Decisions => rsx! { DecisionsTab {} },
                    Tab::Events => rsx! { EventLog { events: events() } },
                }
            }
        }
    }
}

#[component]
fn Placeholder(label: String) -> Element {
    rsx! {
        div { class: "flex h-full items-center justify-center text-neutral-600", "{label}" }
    }
}

#[component]
fn DeliverablesTab() -> Element {
    let task = selection().active_task;
    let deliverables = use_resource(move || async move {
        match task() {
            Some(tid) => api::get_task_deliverables(&tid).await.ok(),
            None => None,
        }
    });

    rsx! {
        div { class: "h-full overflow-y-auto p-2",
            match &*deliverables.read_unchecked() {
                Some(Some(resp)) if !resp.deliverables.is_empty() => {
                    let items = resp.deliverables.iter().map(|d| {
                        let name = d
                            .rendered_content_path
                            .rsplit('/')
                            .next()
                            .unwrap_or(&d.rendered_content_path)
                            .to_string();
                        let fmt = d.format.clone();
                        let size = d.content_size;
                        let id = d.id.clone();
                        rsx! {
                            li { key: "{id}", class: "flex items-center gap-2 border-b border-neutral-900 py-1",
                                span { class: "rounded bg-neutral-800 px-1 text-xs uppercase", "{fmt}" }
                                span { class: "truncate", "{name}" }
                                span { class: "ml-auto text-xs text-neutral-600", "{size} B" }
                            }
                        }
                    }).collect::<Vec<_>>();
                    rsx! { ul { {items.into_iter()} } }
                }
                Some(Some(_)) => rsx! { div { class: "text-neutral-600", "No deliverables yet" } },
                Some(None) => rsx! { div { class: "text-neutral-600", "Select a task to see deliverables" } },
                None => rsx! { div { class: "text-neutral-500", "Loading…" } },
            }
        }
    }
}

#[component]
fn FilesTab() -> Element {
    let session = selection().session_id;
    let listing = use_resource(move || async move {
        match session() {
            Some(sid) => api::list_workspace_root(&sid).await.ok(),
            None => None,
        }
    });

    rsx! {
        div { class: "h-full overflow-y-auto p-2 font-mono text-xs",
            match &*listing.read_unchecked() {
                Some(Some(seasoned_hand_dto::WorkspaceListing::Dir { entries })) if !entries.is_empty() => {
                    let items = entries.iter().map(|e| {
                        let is_dir = e.kind == "dir";
                        let icon = if is_dir { "📁" } else { "📄" };
                        let name = e.name.clone();
                        let size = e.size.map(|s| format!("{s} B")).unwrap_or_default();
                        rsx! {
                            li { key: "{name}", class: "flex items-center gap-2 py-0.5",
                                span { "{icon}" }
                                span { class: if is_dir { "text-blue-300" } else { "" }, "{name}" }
                                span { class: "ml-auto text-neutral-600", "{size}" }
                            }
                        }
                    }).collect::<Vec<_>>();
                    rsx! { ul { {items.into_iter()} } }
                }
                Some(Some(_)) => rsx! { div { class: "text-neutral-600", "Empty workspace" } },
                Some(None) => rsx! { div { class: "text-neutral-600", "Select a session to browse files" } },
                None => rsx! { div { class: "text-neutral-500", "Loading…" } },
            }
        }
    }
}

#[component]
fn VerifierTab() -> Element {
    let session = selection().session_id;
    let verifications = use_resource(move || async move {
        match session() {
            Some(sid) => api::list_verifications(&sid, 50).await.ok(),
            None => None,
        }
    });

    rsx! {
        div { class: "h-full overflow-y-auto p-2 text-xs",
            match &*verifications.read_unchecked() {
                Some(Some(resp)) if !resp.rows.is_empty() => {
                    let items = resp.rows.iter().map(|v| {
                        let pass = v.verdict == seasoned_hand_dto::Verdict::Pass;
                        let (badge, badge_cls) = if pass {
                            ("PASS", "rounded bg-green-700 px-1")
                        } else {
                            ("FAIL", "rounded bg-red-700 px-1")
                        };
                        let kind = v.trigger_kind.clone();
                        let reason = v.reason.clone();
                        let id = v.id.clone();
                        rsx! {
                            li { key: "{id}", class: "border-b border-neutral-900 py-1",
                                div { class: "flex items-center gap-2",
                                    span { class: "{badge_cls}", "{badge}" }
                                    span { class: "text-neutral-400", "{kind}" }
                                }
                                div { class: "mt-0.5 whitespace-pre-wrap break-words text-neutral-300", "{reason}" }
                            }
                        }
                    }).collect::<Vec<_>>();
                    rsx! { ul { class: "space-y-1", {items.into_iter()} } }
                }
                Some(Some(_)) => rsx! { div { class: "text-neutral-600", "No verifications yet" } },
                Some(None) => rsx! { div { class: "text-neutral-600", "Select a session to see verifications" } },
                None => rsx! { div { class: "text-neutral-500", "Loading…" } },
            }
        }
    }
}

/// A decision event is a `Misc` event tagged `decision` (Initializer / Verifier
/// / Checkpoint emit these). Derived from the live event stream, no endpoint.
fn is_decision(ev: &ServerEvent) -> bool {
    ev.payload.get("kind").and_then(|v| v.as_str()) == Some("Misc")
        && ev.payload.get("kind_tag").and_then(|v| v.as_str()) == Some("decision")
}

#[component]
fn DecisionsTab() -> Element {
    let sel = selection();
    let sock = socket();
    let session = sel.session_id;
    let events = sock.events;

    rsx! {
        div { class: "h-full overflow-y-auto p-2 text-xs",
            {
                let sid = session();
                let evs = events();
                if sid.is_none() {
                    rsx! { div { class: "text-neutral-600", "Select a session to view decisions" } }
                } else {
                    let rows = evs
                        .iter()
                        .rev()
                        .filter(|e| sid.as_ref() == Some(&e.session_id) && is_decision(e))
                        .map(|e| {
                            let source = e
                                .payload
                                .get("source")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            let reason = e
                                .payload
                                .get("reason")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let id = e.id.clone();
                            rsx! {
                                li { key: "{id}", class: "border-b border-neutral-900 py-1",
                                    div { class: "text-blue-400", "{source}" }
                                    div { class: "whitespace-pre-wrap break-words text-neutral-300", "{reason}" }
                                }
                            }
                        })
                        .collect::<Vec<_>>();
                    if rows.is_empty() {
                        rsx! { div { class: "text-neutral-600", "No decisions recorded yet" } }
                    } else {
                        rsx! { ul { class: "space-y-1", {rows.into_iter()} } }
                    }
                }
            }
        }
    }
}

#[component]
fn EventLog(events: Vec<ServerEvent>) -> Element {
    rsx! {
        div { class: "h-full overflow-y-auto p-2 font-mono text-xs",
            for e in events {
                div { key: "{e.id}", class: "border-b border-neutral-900 py-0.5",
                    span { class: "mr-2 text-neutral-600", "#{e.id}" }
                    span { class: "mr-2 text-blue-400", "{e.kind().unwrap_or(\"?\")}" }
                    span { class: "text-neutral-400", "{e.payload}" }
                }
            }
        }
    }
}
