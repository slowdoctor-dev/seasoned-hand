# Story 0.22 — TaskList component

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: 0.19 (layout), 0.20 (WS client)
> **Phase**: 0
> **Type**: frontend
> **Reads first**: `/specs/phase-0/architecture.md` §4.1 (`GET /v1/sessions`)

## Goal

Left panel: list of recent sessions, click to make one active. New
session created from the Chat input (story 0.21) but visible here
immediately via WS events.

## Acceptance criteria

- [ ] `<TaskList activeSessionId={...} onSelect={(id)=>...} />`
- [ ] On mount, fetches `GET /v1/sessions?limit=50` (REST, not WS) and
      lists newest first
- [ ] Subscribes via WS to ALL listed sessions so live state changes
      (state, title, cost_cents) update without manual refresh —
      Phase 0 simplification: just re-fetch on any new `Misc` event
      with a fresh session_id (DEBT entry for the cheap polling-via-WS
      approach)
- [ ] Each row: title (or input prefix if no title), state badge
      (`IDLE/RUNNING/FINISHED/ERROR/SUSPENDED`), cost in dollars
- [ ] Active session highlighted (border-left blue)
- [ ] Empty state: "No tasks yet — type one in the center panel"
- [ ] `pnpm typecheck / lint / build` pass

## Non-goals

- Pagination beyond first 50 (Phase 1)
- Session search/filter (Phase 1)
- Deleting sessions (Phase 1 — Phase 0 keeps everything)

## Files changed

- `frontend/components/task-list.tsx` (new)
- `frontend/lib/api.ts` (new — minimal `fetch` wrapper for `/v1/...`)
- `frontend/components/panels/task-list-placeholder.tsx` → real

## Spec references

- `/specs/phase-0/architecture.md` §4.1

## Commit message

```
feat(phase-0): story 0.22 - TaskList component

- Fetches /v1/sessions on mount; subscribes to all via WS for state
  updates (re-fetches on any new Misc event with a fresh session_id —
  cheap Phase-0 invalidation)
- Row: title, state badge, cost; active session highlighted
- pnpm typecheck/lint/build pass

Debt: TaskList re-fetches whole list on any new session_id; replace
with targeted updates in Phase 1.

refs: /specs/phase-0/stories/story-0.22.md
```
