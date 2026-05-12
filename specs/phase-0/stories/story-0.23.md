# Story 0.23 — AgentComputer tabs scaffold

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 0.19 (layout)
> **Phase**: 0
> **Type**: frontend

## Goal

Right panel: a tab strip with four tabs — **Browser**, **Terminal**,
**Editor**, **Files**. Each tab is a placeholder for the real
integrations landing in 0.24 / 0.25 / 0.26 (Files is Phase 1).

## Acceptance criteria

- [ ] `<AgentComputer sessionId={id|null}/>` with tab strip + content area
- [ ] Tabs: Browser (active by default), Terminal, Editor, Files
      (Files marked disabled "Phase 1")
- [ ] Active tab indicator (Tailwind underline)
- [ ] Each tab renders a placeholder that shows the session id and
      "Story 0.24/0.25/0.26 lands here"
- [ ] Selected tab persists per session in `sessionStorage` key
      `sh.tab.{session_id}`
- [ ] `pnpm typecheck / lint / build` pass

## Non-goals

- Real Browser / Terminal / Editor integrations (next 3 stories)
- Splitting AgentComputer into multiple subpanels (Phase 1+)

## Files changed

- `frontend/components/agent-computer.tsx` (new)
- `frontend/components/panels/agent-computer-placeholder.tsx` → wraps real

## Spec references

- `/specs/phase-0/architecture.md` §1 (AgentComputer tabs)

## Commit message

```
feat(phase-0): story 0.23 - AgentComputer tabs scaffold

- Tab strip: Browser (default), Terminal, Editor, Files (disabled)
- Per-session active tab persistence via sessionStorage
- Placeholders for 0.24/0.25/0.26
- pnpm typecheck/lint/build pass

refs: /specs/phase-0/stories/story-0.23.md
```
