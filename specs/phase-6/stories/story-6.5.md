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
