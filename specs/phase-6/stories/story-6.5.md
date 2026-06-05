# Story 6.5 — Desktop + mobile targets

> **Status**: ready

Light up the multi-platform reach that motivated ADR-016, from the same codebase.

## Acceptance criteria

- [ ] **Desktop** (`dioxus-desktop`, webview): builds and runs; Monaco/xterm/
      noVNC interop works in the webview context (same shims as web).
- [ ] **Mobile** (`dioxus-mobile`, iOS + Android): builds; the AgentComputer
      degrades to a **read-only** status/log view (no terminal/editor/VNC) per
      ADR-016.
- [ ] Platform selection behind cargo features (`web` default; `desktop`,
      `mobile`); shared component tree, platform-specific shims isolated.
- [ ] Document build/run steps per platform in `docs/`.

## Notes

`dioxus-mobile` is the least mature target — staged last, behind the desktop +
web wins. If a target slips, it does not block the cutover (6.6) for web/desktop.

**Transport abstraction is a prerequisite.** The UI currently uses `gloo-net`
(wasm `fetch`/WebSocket) and `web-sys` (`window().location`) directly in
`api.rs` / `ws.rs` / `config.rs` — these are **web-only**. Desktop/mobile need a
native transport (e.g. `reqwest` + `tokio-tungstenite`) behind a cargo-feature
seam before they can compile, plus the system webview libs (webkit2gtk) on the
build host. So this is real refactoring work, not a feature-flag flip.
