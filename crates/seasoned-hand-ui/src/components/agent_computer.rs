//! AgentComputer panel (replaces `agent-computer.tsx` + its tab set). Hosts the
//! JS-interop surfaces (browser/terminal/editor), the browser-track visualizers
//! (issue #3: DOM-text pane, screenshot strip + lightbox, evidence chips), a
//! recursive workspace file tree, deliverables/verifier/decisions, and a raw
//! event log.

use super::browser_track::BrowserTab;
use super::evidence_chip::EvidenceChip;
use super::{selection, socket};
use crate::api;
use crate::interop::{MonacoEditor, XtermTerminal};
use dioxus::prelude::*;
use seasoned_hand_dto::{ServerEvent, Verification, WorkspaceListing};
use std::collections::HashMap;

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

/// A file opened from the Files tab into the Editor tab.
#[derive(Clone, PartialEq)]
struct OpenFile {
    path: String,
    content: String,
}

/// Panel-local shared state so the Files tree can drive the active tab + the
/// editor's open file. `Copy` (signal handles), shared via context.
#[derive(Clone, Copy)]
struct AgentComputerCtx {
    tab: Signal<Tab>,
    open_file: Signal<Option<OpenFile>>,
}

/// Map a path's extension to a Monaco language id (subset; mirrors
/// `frontend/lib/workspace.ts::languageForPath`).
fn language_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("rs") => "rust",
        Some("ts") | Some("tsx") => "typescript",
        Some("js") | Some("jsx") => "javascript",
        Some("json") => "json",
        Some("md") => "markdown",
        Some("py") => "python",
        Some("go") => "go",
        Some("yaml") | Some("yml") => "yaml",
        Some("toml") => "toml",
        Some("sql") => "sql",
        Some("sh") => "shell",
        Some("html") => "html",
        Some("css") => "css",
        _ => "plaintext",
    }
}

#[component]
pub fn AgentComputer() -> Element {
    let sel = selection();
    let sock = socket();
    let session = sel.session_id;
    let events = sock.events;
    let mut tab = use_signal(|| Tab::Browser);
    let open_file = use_signal(|| Option::<OpenFile>::None);
    // Shared so the Files tree can open a file into the Editor tab.
    use_context_provider(|| AgentComputerCtx { tab, open_file });

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
                    Tab::Browser => match (&sandbox, session()) {
                        (Some(s), Some(sid)) => rsx! {
                            BrowserTab { session_id: sid, novnc_url: s.novnc_url.clone() }
                        },
                        _ => rsx! { Placeholder { label: "No sandbox / browser for this session" } },
                    },
                    Tab::Terminal => match &sandbox {
                        Some(s) => rsx! { XtermTerminal { ws_url: s.ttyd_url.clone() } },
                        None => rsx! { Placeholder { label: "No terminal for this session" } },
                    },
                    Tab::Editor => match open_file() {
                        Some(f) => rsx! {
                            // Reactive interop (issue #3): switching files swaps the live
                            // editor's model in place — no key-forced re-mount.
                            MonacoEditor {
                                value: f.content.clone(),
                                language: language_for_path(&f.path).to_string(),
                            }
                        },
                        None => rsx! { Placeholder { label: "Select a file in the Files tab" } },
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
            match (session(), &*listing.read_unchecked()) {
                (Some(sid), Some(Some(WorkspaceListing::Dir { entries }))) if !entries.is_empty() => {
                    let nodes = entries.iter().map(|e| {
                        rsx! {
                            FileNode {
                                key: "{e.name}",
                                session_id: sid.clone(),
                                path: e.name.clone(),
                                name: e.name.clone(),
                                is_dir: e.kind == "dir",
                                depth: 0,
                            }
                        }
                    }).collect::<Vec<_>>();
                    rsx! { div { {nodes.into_iter()} } }
                }
                (Some(_), Some(Some(_))) => rsx! { div { class: "text-neutral-600", "Empty workspace" } },
                (None, _) => rsx! { div { class: "text-neutral-600", "Select a session to browse files" } },
                (_, None) => rsx! { div { class: "text-neutral-500", "Loading…" } },
                _ => rsx! { div { class: "text-neutral-600", "Empty workspace" } },
            }
        }
    }
}

/// One row in the workspace tree. Directories expand on click (lazily fetching
/// their children); files open into the Editor tab via [`AgentComputerCtx`].
#[component]
fn FileNode(session_id: String, path: String, name: String, is_dir: bool, depth: usize) -> Element {
    let ctx = use_context::<AgentComputerCtx>();
    let mut expanded = use_signal(|| false);
    let indent = format!("padding-left: {}px", depth * 12);

    let children = {
        let session_id = session_id.clone();
        let path = path.clone();
        use_resource(move || {
            let session_id = session_id.clone();
            let path = path.clone();
            async move {
                if is_dir && expanded() {
                    api::list_workspace_dir(&session_id, &path).await.ok()
                } else {
                    None
                }
            }
        })
    };

    let on_click = {
        let session_id = session_id.clone();
        let path = path.clone();
        let mut tab = ctx.tab;
        let mut open_file = ctx.open_file;
        move |_| {
            if is_dir {
                let now = expanded();
                expanded.set(!now);
            } else {
                let session_id = session_id.clone();
                let path = path.clone();
                spawn(async move {
                    if let Ok(content) = api::read_workspace_file(&session_id, &path).await {
                        open_file.set(Some(OpenFile {
                            path: path.clone(),
                            content,
                        }));
                        tab.set(Tab::Editor);
                    }
                });
            }
        }
    };

    let icon = if is_dir {
        if expanded() {
            "▾ 📁"
        } else {
            "▸ 📁"
        }
    } else {
        "📄"
    };

    rsx! {
        div {
            button {
                style: "{indent}",
                class: "flex w-full items-center gap-1 rounded px-1 py-0.5 text-left hover:bg-neutral-900",
                onclick: on_click,
                span { "{icon}" }
                span { class: if is_dir { "text-blue-300" } else { "" }, "{name}" }
            }
            if is_dir && expanded() {
                match &*children.read_unchecked() {
                    Some(Some(WorkspaceListing::Dir { entries })) => {
                        let kids = entries.iter().map(|e| {
                            let child_path = if path.is_empty() {
                                e.name.clone()
                            } else {
                                format!("{path}/{}", e.name)
                            };
                            rsx! {
                                FileNode {
                                    key: "{child_path}",
                                    session_id: session_id.clone(),
                                    path: child_path.clone(),
                                    name: e.name.clone(),
                                    is_dir: e.kind == "dir",
                                    depth: depth + 1,
                                }
                            }
                        }).collect::<Vec<_>>();
                        rsx! { div { {kids.into_iter()} } }
                    }
                    Some(None) => rsx! { div { style: "padding-left: {(depth + 1) * 12}px", class: "text-red-400", "load failed" } },
                    _ => rsx! {},
                }
            }
        }
    }
}

/// Per-session event index for the evidence chips' O(1) lookup (issue #3;
/// parity with the React `HomeShell`'s `eventIndex`). Keyed by numeric event
/// id over the currently loaded WS window.
fn build_event_index(events: &[ServerEvent], session_id: &str) -> HashMap<i64, ServerEvent> {
    let mut map = HashMap::new();
    for ev in events {
        if ev.session_id != session_id {
            continue;
        }
        if let Ok(id) = ev.id.parse::<i64>() {
            map.insert(id, ev.clone());
        }
    }
    map
}

#[component]
fn VerifierTab() -> Element {
    let session = selection().session_id;
    let events = socket().events;
    // React parity: every new verifier_verdict Misc event for this session bumps
    // this count, which the resource reads — so the list re-fetches from the
    // canonical HTTP endpoint instead of reconciling client-side state.
    let verdict_count = use_memo(move || match session() {
        Some(sid) => events()
            .iter()
            .filter(|e| {
                e.session_id == sid
                    && e.kind() == Some("Misc")
                    && e.payload.get("kind_tag").and_then(|v| v.as_str())
                        == Some("verifier_verdict")
            })
            .count(),
        None => 0,
    });
    let verifications = use_resource(move || async move {
        let _refresh_on_new_verdict = verdict_count();
        match session() {
            Some(sid) => api::list_verifications(&sid, 50).await.ok(),
            None => None,
        }
    });
    let event_index = use_memo(move || match session() {
        Some(sid) => build_event_index(&events(), &sid),
        None => HashMap::new(),
    });

    rsx! {
        div { class: "h-full overflow-y-auto p-2 text-xs",
            match &*verifications.read_unchecked() {
                Some(Some(resp)) if !resp.rows.is_empty() => {
                    let items = resp.rows.iter().map(|v| {
                        rsx! {
                            VerdictRow { key: "{v.id}", verdict: v.clone(), event_index }
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

/// One verdict row: badge + reason, expanding to evidence chips (resolved via
/// the per-session event index) and the optional suggested plan update.
#[component]
fn VerdictRow(verdict: Verification, event_index: Memo<HashMap<i64, ServerEvent>>) -> Element {
    let mut expanded = use_signal(|| false);
    let pass = verdict.verdict == seasoned_hand_dto::Verdict::Pass;
    let (badge, badge_cls) = if pass {
        ("PASS", "rounded bg-green-700 px-1")
    } else {
        ("FAIL", "rounded bg-red-700 px-1")
    };
    let kind = verdict.trigger_kind.clone();
    let model = verdict.model_id.clone();
    let reason = verdict.reason.clone();
    let chevron = if expanded() { "▾" } else { "▸" };
    let plan_update = verdict
        .suggested_plan_update
        .as_ref()
        .map(|v| serde_json::to_string_pretty(v).unwrap_or_default());

    rsx! {
        li { class: "border-b border-neutral-900 py-1",
            button {
                class: "flex w-full items-start gap-2 text-left hover:bg-neutral-900",
                onclick: move |_| {
                    let now = expanded();
                    expanded.set(!now);
                },
                span { class: "{badge_cls}", "{badge}" }
                div { class: "flex-1",
                    div { class: "whitespace-pre-wrap break-words text-neutral-300", "{reason}" }
                    div { class: "mt-0.5 text-neutral-500", "{kind} · {model}" }
                }
                span { class: "text-neutral-500", "{chevron}" }
            }
            if expanded() {
                div { class: "mt-1 space-y-2 rounded bg-neutral-900 p-2",
                    if !verdict.evidence_event_ids.is_empty() {
                        div {
                            div { class: "mb-1 text-neutral-500", "Evidence:" }
                            div { class: "flex flex-wrap gap-1",
                                for id in verdict.evidence_event_ids.iter() {
                                    EvidenceChip {
                                        key: "{id}",
                                        event_id: *id,
                                        event: event_index().get(id).cloned(),
                                    }
                                }
                            }
                        }
                    }
                    if let Some(plan) = plan_update.as_ref() {
                        div {
                            div { class: "mb-1 text-neutral-500", "Suggested plan update:" }
                            pre { class: "max-h-40 overflow-auto rounded bg-neutral-950 p-2 text-[10px]",
                                "{plan}"
                            }
                        }
                    }
                    if verdict.evidence_event_ids.is_empty() && plan_update.is_none() {
                        div { class: "text-neutral-600", "No evidence attached" }
                    }
                }
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
                            // Misc payloads nest the emitter's fields under `data`
                            // (ws build_payload); fall back to the top level for
                            // older flattened shapes.
                            let field = |name: &str| {
                                e.payload
                                    .get("data")
                                    .and_then(|d| d.get(name))
                                    .or_else(|| e.payload.get(name))
                                    .and_then(|v| v.as_str())
                                    .map(String::from)
                            };
                            let source = field("source").unwrap_or_else(|| "unknown".into());
                            let reason = field("reason").unwrap_or_default();
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
