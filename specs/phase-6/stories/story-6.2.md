# Story 6.2 — Interop spike: real Monaco / xterm / noVNC bundles

> **Status**: ready

**This is the ADR-016 step-1 de-risking gate.** Replace the no-op `window.__mount*`
shims in `index.html` with real library initialization and prove the three
JS-only components work end-to-end inside Dioxus web.

## Acceptance criteria

- [ ] Vendor (or CDN-load) `monaco-editor`, `@xterm/xterm` (+ fit/attach addons),
      and `noVNC`; wire each `window.__mount*` shim to real init targeting the
      Rust-rendered mount `<div id>`.
- [ ] Monaco renders a read-only document; xterm attaches to a ttyd socket;
      noVNC connects to a session's `novnc_url`.
- [ ] Mount/unmount lifecycle is clean on tab switch (no leaked instances).
- [ ] Verified against a live session in a Docker-enabled environment.

## Gate

If interop proves unworkable, **stop the full migration** and fall back to the
ADR-016 companion option (keep Next.js, Dioxus for status/approvals only). Record
the outcome in ADR-016.
