# Story 0.24 — noVNC iframe integration

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 0.8 (sandbox client), 0.17 (WS — to learn the active session's sandbox URL), 0.23 (AgentComputer tabs)
> **Phase**: 0
> **Type**: frontend

## Goal

Render the running sandbox's noVNC in the Browser tab via `<iframe>`.
The frontend learns the sandbox `novnc_url` from the session detail
endpoint (added here).

## Acceptance criteria

- [ ] `GET /v1/sessions/:id` extended to include
      `sandbox: { novnc_url, ttyd_url, api_url } | null` if a sandbox
      is running for that session
- [ ] `<BrowserTab sessionId={id}/>` calls the endpoint, renders an
      `<iframe src={novnc_url}>` filling the tab body
- [ ] If no sandbox yet for the session: empty state "Sandbox starts
      when the agent calls its first browser/shell/file tool"
- [ ] iframe attributes: `sandbox="allow-scripts allow-same-origin"`,
      explicit `width="100%"` + `height="100%"`
- [ ] Refresh button reloads the iframe (re-key React element)
- [ ] Tab "info" overlay shows session id + url for debugging
- [ ] `pnpm typecheck / lint / build` pass

## Non-goals

- Takeover (architecture §3.10 mentions toggle; Phase 0 is info-only)
- Embedded VNC controls (full noVNC handles inside the iframe)

## Files changed

- `crates/seasoned-hand-server/src/lib.rs` (extend `GET /v1/sessions/:id`)
- `frontend/components/agent-computer/browser-tab.tsx` (new)
- `frontend/lib/api.ts` (add `getSession(id)` helper)

## Spec references

- `/specs/phase-0/architecture.md` §1 (right panel — Browser via noVNC)
- `/specs/01-architecture/decisions/ADR-004-aio-sandbox-per-session.md`

## Commit message

```
feat(phase-0): story 0.24 - noVNC iframe in Browser tab

- GET /v1/sessions/:id now returns sandbox.{novnc_url,ttyd_url,api_url}
  when a sandbox is running for the session, else null
- BrowserTab iframes novnc_url with sandbox attrs, empty state when
  no sandbox yet, refresh button via React key bump
- pnpm typecheck/lint/build pass

refs: /specs/phase-0/stories/story-0.24.md
```
