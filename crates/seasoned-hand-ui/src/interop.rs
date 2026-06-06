//! JS-interop wrappers for the three browser-only libraries with no native Rust
//! equivalent: **Monaco** (editor), **xterm.js** (terminal), and **noVNC**
//! (browser takeover). This is the load-bearing risk ADR-016 flagged — proving
//! these survive in Dioxus via interop is the gate for the full migration.
//!
//! Approach: each wrapper renders a stable mount `<div id>` and, on mount, calls
//! a small `window.__mount*` shim (defined in `index.html`, which loads the JS
//! libs from a CDN/vendor bundle) through `document::eval`. The Rust side never
//! reimplements the libraries — it owns the lifecycle and passes data in.
//!
//! Foundation scope: the wrappers mount once and pass initial data. Reactive
//! updates (re-pushing terminal output, swapping editor models) and teardown are
//! Phase 6 follow-ups; the shims are intentionally no-ops if the lib is absent,
//! so the panel renders cleanly even before the vendor bundle is wired.

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

/// Read-only Monaco editor mount.
#[component]
pub fn MonacoEditor(value: String, language: String) -> Element {
    let id = use_hook(|| next_dom_id("monaco"));
    {
        let id = id.clone();
        use_effect(move || {
            let script = format!(
                "if (window.__mountMonaco) {{ window.__mountMonaco({:?}, {:?}, {:?}); }}",
                id, value, language
            );
            let _ = document::eval(&script);
        });
    }
    rsx! { div { id: "{id}", class: "h-full w-full" } }
}

/// xterm.js terminal mount. `ws_url` is the ttyd/terminal socket the shim
/// attaches the terminal to.
#[component]
pub fn XtermTerminal(ws_url: String) -> Element {
    let id = use_hook(|| next_dom_id("xterm"));
    {
        let id = id.clone();
        use_effect(move || {
            let script = format!(
                "if (window.__mountXterm) {{ window.__mountXterm({:?}, {:?}); }}",
                id, ws_url
            );
            let _ = document::eval(&script);
        });
    }
    rsx! { div { id: "{id}", class: "h-full w-full" } }
}

/// noVNC browser-takeover mount.
#[component]
pub fn NoVnc(novnc_url: String) -> Element {
    let id = use_hook(|| next_dom_id("novnc"));
    {
        let id = id.clone();
        use_effect(move || {
            let script = format!(
                "if (window.__mountNoVnc) {{ window.__mountNoVnc({:?}, {:?}); }}",
                id, novnc_url
            );
            let _ = document::eval(&script);
        });
    }
    rsx! { div { id: "{id}", class: "h-full w-full" } }
}
