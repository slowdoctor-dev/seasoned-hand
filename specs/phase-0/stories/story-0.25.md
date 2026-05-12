# Story 0.25 — xterm.js + ttyd terminal

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 0.24 (sandbox URLs surfaced), 0.23 (tabs)
> **Phase**: 0
> **Type**: frontend

## Goal

Terminal tab: xterm.js connected to the sandbox's ttyd WebSocket.
Read-only by default (no keyboard input) — full interactive ttyd
lands in Phase 1.

## Acceptance criteria

- [ ] `frontend/package.json` adds `@xterm/xterm`, `@xterm/addon-fit`,
      `@xterm/addon-attach`
- [ ] `<TerminalTab sessionId={id}/>` opens a WS to `ttyd_url` from
      the session detail
- [ ] xterm canvas fills the tab body; fit addon resizes on container
      resize (use `ResizeObserver`)
- [ ] **Read-only**: xterm's `disableStdin: true`; keyboard events do
      not propagate to the ttyd socket
- [ ] If `ttyd_url` is missing (no sandbox): empty state matching 0.24
- [ ] Reconnect on close with the same backoff curve as `useAgentSocket`
- [ ] `pnpm typecheck / lint / build` pass

## Non-goals

- Read-write terminal (Phase 1)
- Multiple terminal panes (Phase 1+)
- Copy/paste polish (Phase 1)

## Files changed

- `frontend/package.json`
- `frontend/components/agent-computer/terminal-tab.tsx` (new)

## Spec references

- `/specs/phase-0/architecture.md` §1 (xterm + ttyd)

## Commit message

```
feat(phase-0): story 0.25 - xterm.js terminal (read-only)

- @xterm/xterm + fit + attach addons
- TerminalTab connects to session's ttyd_url, disableStdin so the
  Phase 0 terminal is observe-only (interactive ttyd = Phase 1)
- ResizeObserver-driven fit; reconnect with backoff
- pnpm typecheck/lint/build pass

refs: /specs/phase-0/stories/story-0.25.md
```
