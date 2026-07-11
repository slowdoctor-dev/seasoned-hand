//! Browser-track visualizers (issue #3, ports the removed React
//! `agent-computer/{browser-tab,dom-text-pane,screenshot-strip,lightbox}.tsx`).
//!
//! Track B (`Misc{kind_tag:"browser_track_b"}`) carries a `dom_text_ref` —
//! inline bytes or a workspace file_ref — rendered by [`DomTextPane`].
//! Track C (`Misc{kind_tag:"browser_track_c"}`) carries a PNG `file_ref` under
//! `/workspace/.tracks/`; [`ScreenshotStrip`] renders a thumbnail strip with a
//! click-to-open [`Lightbox`]. The workspace proxy is auth-gated (ADR-018), so
//! images are fetched with the bearer token and shown via `data:` URLs rather
//! than raw `<img src>` proxy links.

use super::socket;
use crate::api;
use dioxus::prelude::*;
use seasoned_hand_dto::ServerEvent;

/// Cap on live-derived thumbnails, matching the React strip. Older shots can
/// be pulled in from `/workspace/.tracks/` with the "load older" button.
const MAX_VISIBLE: usize = 100;

/// One strip entry: a captured screenshot or a skip marker.
#[derive(Clone, PartialEq)]
enum Shot {
    Ok { key: String, path: String },
    Skipped { key: String, reason: String },
}

/// Misc payloads arrive as `{kind:"Misc", kind_tag, data:{…}}` (ws.rs
/// `build_payload`); read a field from `data`, falling back to the top level
/// for older flattened payload shapes.
fn misc_field<'a>(payload: &'a serde_json::Value, field: &str) -> Option<&'a serde_json::Value> {
    payload
        .get("data")
        .and_then(|d| d.get(field))
        .or_else(|| payload.get(field))
}

fn kind_tag(ev: &ServerEvent) -> Option<&str> {
    if ev.kind() != Some("Misc") {
        return None;
    }
    ev.payload.get("kind_tag").and_then(|v| v.as_str())
}

/// Derive the Track C strip from the live event window. Returns the (capped)
/// shots plus how many older entries fell off the cap.
fn track_c_shots(events: &[ServerEvent], session_id: &str) -> (Vec<Shot>, usize) {
    let mut rows: Vec<Shot> = Vec::new();
    let mut hidden = 0usize;
    for ev in events {
        if ev.session_id != session_id {
            continue;
        }
        let shot = match kind_tag(ev) {
            Some("browser_track_c") => {
                let call_id = misc_field(&ev.payload, "call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                match misc_field(&ev.payload, "file_ref")
                    .and_then(|f| f.get("path"))
                    .and_then(|v| v.as_str())
                {
                    Some(path) => Shot::Ok {
                        key: format!("{}:{call_id}", ev.id),
                        path: path.to_string(),
                    },
                    None => continue,
                }
            }
            Some("browser_track_c_skipped") => {
                let call_id = misc_field(&ev.payload, "call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let reason = misc_field(&ev.payload, "reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                Shot::Skipped {
                    key: format!("{}:{call_id}", ev.id),
                    reason: reason.to_string(),
                }
            }
            _ => continue,
        };
        rows.push(shot);
        if rows.len() > MAX_VISIBLE {
            rows.remove(0);
            hidden += 1;
        }
    }
    (rows, hidden)
}

/// Latest Track B `dom_text_ref` for the session, if any.
fn latest_dom_text_ref(events: &[ServerEvent], session_id: &str) -> Option<serde_json::Value> {
    events
        .iter()
        .rev()
        .find(|ev| ev.session_id == session_id && kind_tag(ev) == Some("browser_track_b"))
        .and_then(|ev| misc_field(&ev.payload, "dom_text_ref").cloned())
}

/// Minimal base64 encoder for `data:` URLs (standard alphabet, padded). Kept
/// local to avoid a new dependency for ~20 lines of table lookup.
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Event payloads carry in-container paths (`/workspace/.tracks/x.png`), but
/// the workspace proxy resolves sub-paths against the workspace ROOT (the host
/// dir bind-mounted at `/workspace`) — strip the mount prefix or the proxy
/// would look for a literal `workspace/` subdirectory.
fn workspace_rel(path: &str) -> &str {
    path.strip_prefix("/workspace/")
        .or_else(|| path.strip_prefix("/workspace"))
        .unwrap_or(path)
}

async fn fetch_data_url(session_id: &str, path: &str) -> Option<String> {
    let bytes = api::read_workspace_file_bytes(session_id, workspace_rel(path))
        .await
        .ok()?;
    Some(format!("data:image/png;base64,{}", base64_encode(&bytes)))
}

/// Browser tab body: noVNC live view on top, Track B / Track C panes below
/// (parity with the React `browser-tab.tsx` layout).
#[component]
pub fn BrowserTab(session_id: String, novnc_url: String) -> Element {
    rsx! {
        div { class: "flex h-full flex-col gap-2 p-2",
            div { class: "min-h-0 flex-[3] overflow-hidden rounded border border-neutral-800",
                crate::interop::NoVnc { novnc_url }
            }
            div { class: "grid min-h-0 flex-[2] grid-cols-2 gap-2",
                div { class: "min-h-0 overflow-hidden rounded border border-neutral-800 p-2",
                    div { class: "mb-1 text-[11px] text-neutral-500", "Track B · DOM text" }
                    DomTextPane { session_id: session_id.clone() }
                }
                div { class: "min-h-0 overflow-hidden rounded border border-neutral-800 p-2",
                    ScreenshotStrip { session_id }
                }
            }
        }
    }
}

/// Track B pane: renders the most recent DOM-text snapshot (inline bytes
/// decoded directly; file_refs fetched from the workspace proxy).
#[component]
pub fn DomTextPane(session_id: String) -> Element {
    let events = socket().events;
    let sid = session_id.clone();
    let latest = use_memo(move || latest_dom_text_ref(&events(), &sid));

    let text = use_resource(move || {
        let session_id = session_id.clone();
        async move {
            let dom_ref = latest()?;
            match dom_ref.get("kind").and_then(|v| v.as_str()) {
                Some("inline") => {
                    let bytes: Vec<u8> = dom_ref
                        .get("bytes")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default();
                    Some(String::from_utf8_lossy(&bytes).into_owned())
                }
                Some("file_ref") => {
                    let path = dom_ref.get("path").and_then(|v| v.as_str())?;
                    api::read_workspace_file(&session_id, workspace_rel(path))
                        .await
                        .ok()
                }
                _ => None,
            }
        }
    });

    let body = match &*text.read_unchecked() {
        Some(Some(t)) if !t.is_empty() => t.clone(),
        _ => "(no DOM snapshot yet)".to_string(),
    };
    rsx! {
        pre { class: "h-full overflow-auto whitespace-pre-wrap font-mono text-xs text-neutral-300",
            "{body}"
        }
    }
}

/// Track C strip: thumbnails of captured screenshots (+ skip markers), a
/// "load older" backfill from `/workspace/.tracks/`, and a lightbox.
#[component]
pub fn ScreenshotStrip(session_id: String) -> Element {
    let events = socket().events;
    let mut older = use_signal(Vec::<Shot>::new);
    let mut older_loaded = use_signal(|| false);
    let mut open_path = use_signal(|| Option::<String>::None);

    let sid = session_id.clone();
    let live = use_memo(move || track_c_shots(&events(), &sid));
    let (live_shots, hidden_in_live) = live();

    let mut shots = older();
    shots.extend(live_shots);
    let show_load_older = hidden_in_live > 0 && !older_loaded();

    let load_older = {
        let session_id = session_id.clone();
        move |_| {
            let session_id = session_id.clone();
            let known: Vec<String> = shots_file_names(&older(), &live().0);
            spawn(async move {
                if let Ok(seasoned_hand_dto::WorkspaceListing::Dir { entries }) =
                    api::list_workspace_dir(&session_id, ".tracks").await
                {
                    let mut names: Vec<String> = entries
                        .iter()
                        .filter(|e| e.kind == "file" && e.name.ends_with(".png"))
                        .map(|e| e.name.clone())
                        .filter(|n| !known.contains(n))
                        .collect();
                    names.sort();
                    names.truncate(50);
                    let fresh: Vec<Shot> = names
                        .into_iter()
                        .map(|name| Shot::Ok {
                            key: format!("older:{name}"),
                            path: format!("/workspace/.tracks/{name}"),
                        })
                        .collect();
                    let mut cur = older();
                    let mut next = fresh;
                    next.append(&mut cur);
                    older.set(next);
                }
                older_loaded.set(true);
            });
        }
    };

    rsx! {
        div { class: "flex h-full flex-col gap-1",
            div { class: "flex items-center justify-between text-[11px] text-neutral-500",
                span { "Screenshots" }
                if show_load_older {
                    button {
                        class: "rounded border border-neutral-700 px-2 py-0.5 hover:bg-neutral-800",
                        onclick: load_older,
                        "older screenshots hidden ({hidden_in_live}) · load 50"
                    }
                }
            }
            div { class: "flex min-h-0 flex-1 gap-1 overflow-x-auto rounded border border-neutral-800 p-1",
                if shots.is_empty() {
                    div { class: "flex w-full items-center justify-center text-xs text-neutral-600",
                        "(no screenshots yet)"
                    }
                }
                for shot in shots.iter() {
                    match shot {
                        Shot::Skipped { key, reason } => rsx! {
                            div {
                                key: "{key}",
                                class: "flex min-w-24 items-center justify-center rounded border border-neutral-800 bg-neutral-900 px-2 text-[10px] text-neutral-500",
                                title: "{reason}",
                                "skipped: {reason}"
                            }
                        },
                        Shot::Ok { key, path } => rsx! {
                            Thumb {
                                key: "{key}",
                                session_id: session_id.clone(),
                                path: path.clone(),
                                on_open: move |p: String| open_path.set(Some(p)),
                            }
                        },
                    }
                }
            }
            if let Some(path) = open_path() {
                Lightbox {
                    session_id: session_id.clone(),
                    path,
                    on_close: move |_| open_path.set(None),
                }
            }
        }
    }
}

/// File names (basename) of every known ok/broken shot, for older-backfill dedup.
fn shots_file_names(older: &[Shot], live: &[Shot]) -> Vec<String> {
    older
        .iter()
        .chain(live.iter())
        .filter_map(|s| match s {
            Shot::Ok { path, .. } => path.rsplit('/').next().map(String::from),
            Shot::Skipped { .. } => None,
        })
        .collect()
}

/// One thumbnail: fetches the PNG with auth and renders a `data:` URL. Broken
/// fetches render a placeholder instead of dropping the slot.
#[component]
fn Thumb(session_id: String, path: String, on_open: EventHandler<String>) -> Element {
    let src = {
        let session_id = session_id.clone();
        let path = path.clone();
        use_resource(move || {
            let session_id = session_id.clone();
            let path = path.clone();
            async move { fetch_data_url(&session_id, &path).await }
        })
    };

    match &*src.read_unchecked() {
        Some(Some(url)) => {
            let url = url.clone();
            let path = path.clone();
            rsx! {
                img {
                    src: "{url}",
                    alt: "browser screenshot",
                    class: "h-full min-w-20 cursor-pointer rounded border border-neutral-800 object-cover",
                    onclick: move |_| on_open.call(path.clone()),
                }
            }
        }
        Some(None) => rsx! {
            div {
                class: "flex h-full min-w-20 items-center justify-center rounded border border-neutral-800 bg-neutral-900 text-lg text-neutral-500",
                title: "image unavailable",
                "⛔"
            }
        },
        None => rsx! {
            div { class: "flex h-full min-w-20 items-center justify-center rounded border border-neutral-800 bg-neutral-900 text-[10px] text-neutral-600",
                "…"
            }
        },
    }
}

/// Full-size overlay for a screenshot. Click the backdrop (or the ✕) to close.
#[component]
fn Lightbox(session_id: String, path: String, on_close: EventHandler<()>) -> Element {
    let src = {
        let session_id = session_id.clone();
        let path = path.clone();
        use_resource(move || {
            let session_id = session_id.clone();
            let path = path.clone();
            async move { fetch_data_url(&session_id, &path).await }
        })
    };

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex items-center justify-center bg-black/80 p-4",
            onclick: move |_| on_close.call(()),
            button {
                class: "absolute right-4 top-4 rounded bg-neutral-800 px-3 py-1 text-neutral-200 hover:bg-neutral-700",
                onclick: move |e| {
                    e.stop_propagation();
                    on_close.call(());
                },
                "✕"
            }
            match &*src.read_unchecked() {
                Some(Some(url)) => rsx! {
                    img {
                        src: "{url}",
                        alt: "browser screenshot fullsize",
                        class: "max-h-full max-w-full rounded border border-white/20 bg-black object-contain",
                        onclick: move |e| e.stop_propagation(),
                    }
                },
                Some(None) => rsx! { div { class: "text-neutral-400", "image unavailable" } },
                None => rsx! { div { class: "text-neutral-400", "Loading…" } },
            }
        }
    }
}
