//! JS-interop wrappers for the three browser-only libraries with no native Rust
//! equivalent: **Monaco** (editor), **xterm.js** (terminal), and **noVNC**
//! (browser takeover). This is the load-bearing risk ADR-016 flagged — proving
//! these survive in Dioxus via interop is the gate for the full migration.
//!
//! Approach: each wrapper renders a stable mount `<div id>` and calls a small
//! `window.__mount*` shim (defined in `index.html`, which loads the JS libs
//! from a CDN/vendor bundle) through `document::eval`. The Rust side never
//! reimplements the libraries — it owns the lifecycle and passes data in.
//!
//! Lifecycle (issue #3 / issue #2): mounts are **reactive** — the effects
//! re-run when props change (`use_reactive`), and the shims are idempotent per
//! mount id (Monaco swaps the model value/language in place; xterm/noVNC
//! dispose the previous instance before re-attaching). `use_drop` calls
//! `window.__disposeInterop(id)` so tab switches don't leak instances.

use dioxus::prelude::*;
use std::cell::Cell;

/// Process-unique DOM id (wasm is single-threaded, so a thread-local counter is
/// sufficient and deterministic per session).
fn next_dom_id(prefix: &str) -> String {
    thread_local! { static COUNTER: Cell<u64> = const { Cell::new(0) }; }
    COUNTER.with(|c| {
        let v = c.get() + 1;
        c.set(v);
        format!("{prefix}-{v}")
    })
}

/// Encode a Rust string as a JS string literal that is safe to embed in a
/// `document::eval` script (issue #22). The wrappers previously interpolated args
/// with `{:?}` (Rust `Debug`), which is NOT a JS/JSON encoder — a Monaco `value`
/// is attacker-influenced workspace content, so a crafted string could break out
/// of the literal and inject script. `serde_json::to_string` emits a valid JSON
/// string literal (a valid JS string); we additionally escape U+2028/U+2029,
/// which are legal in JSON but terminate a JS string literal.
fn js_string(s: &str) -> String {
    let encoded = serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string());
    if encoded.contains('\u{2028}') || encoded.contains('\u{2029}') {
        encoded
            .replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029")
    } else {
        encoded
    }
}

/// Dispose the JS-side instance bound to `id` (editor / terminal+socket / RFB).
fn dispose_interop(id: &str) {
    let script = format!(
        "if (window.__disposeInterop) {{ window.__disposeInterop({}); }}",
        js_string(id)
    );
    let _ = document::eval(&script);
}

/// Read-only Monaco editor mount. Reactive: a changed `value`/`language`
/// re-invokes the shim, which swaps the live editor's model in place (no
/// re-mount, preserving scroll/layout).
#[component]
pub fn MonacoEditor(value: String, language: String) -> Element {
    let id = use_hook(|| next_dom_id("monaco"));
    {
        let id = id.clone();
        use_effect(use_reactive(
            (&value, &language),
            move |(value, language)| {
                let script = format!(
                    "if (window.__mountMonaco) {{ window.__mountMonaco({}, {}, {}); }}",
                    js_string(&id),
                    js_string(&value),
                    js_string(&language)
                );
                let _ = document::eval(&script);
            },
        ));
    }
    {
        let id = id.clone();
        use_drop(move || dispose_interop(&id));
    }
    rsx! { div { id: "{id}", class: "h-full w-full" } }
}

/// xterm.js terminal mount. `ws_url` is the ttyd/terminal socket the shim
/// attaches the terminal to. Reactive: a changed `ws_url` disposes the previous
/// terminal + socket and re-attaches.
#[component]
pub fn XtermTerminal(ws_url: String) -> Element {
    let id = use_hook(|| next_dom_id("xterm"));
    {
        let id = id.clone();
        use_effect(use_reactive((&ws_url,), move |(ws_url,)| {
            let script = format!(
                "if (window.__mountXterm) {{ window.__mountXterm({}, {}); }}",
                js_string(&id),
                js_string(&ws_url)
            );
            let _ = document::eval(&script);
        }));
    }
    {
        let id = id.clone();
        use_drop(move || dispose_interop(&id));
    }
    rsx! { div { id: "{id}", class: "h-full w-full" } }
}

/// noVNC browser-takeover mount. Reactive: a changed `novnc_url` disconnects
/// the previous RFB and reconnects.
#[component]
pub fn NoVnc(novnc_url: String) -> Element {
    let id = use_hook(|| next_dom_id("novnc"));
    {
        let id = id.clone();
        use_effect(use_reactive((&novnc_url,), move |(novnc_url,)| {
            let script = format!(
                "if (window.__mountNoVnc) {{ window.__mountNoVnc({}, {}); }}",
                js_string(&id),
                js_string(&novnc_url)
            );
            let _ = document::eval(&script);
        }));
    }
    {
        let id = id.clone();
        use_drop(move || dispose_interop(&id));
    }
    rsx! { div { id: "{id}", class: "h-full w-full" } }
}
