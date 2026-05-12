# Story 0.20 — WebSocket client + reconnection

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: 0.17 (WS server), 0.18 (frontend), 0.19 (layout)
> **Phase**: 0
> **Type**: frontend
> **Reads first**: `/specs/phase-0/architecture.md` §4.2 (envelope protocol)

## Goal

A typed WS client hook (`useAgentSocket`) that connects to `/ws`,
auto-reconnects with exponential backoff, replays missed events on
reconnect via `subscribe { from_event_id }`, and exposes an
event-stream + command-sender to React components.

## Acceptance criteria

- [ ] `frontend/lib/ws.ts` exports `useAgentSocket(url)` returning
      `{ status, events, send, lastEventId }`
- [ ] `status: "connecting" | "open" | "closed" | "reconnecting"`
- [ ] `events` is an append-only `EventEnvelope[]` (capped at 1000;
      oldest dropped — DEBT note for the cap)
- [ ] Auto-reconnect with backoff `1s → 2s → 4s → 8s` (max 30s);
      resets on successful open
- [ ] On reconnect, re-issues `subscribe { from_event_id: lastEventId }`
      for every previously-subscribed session
- [ ] `send(command)` returns a `Promise<Ack>` keyed by the command's
      `id` (uuid generated client-side)
- [ ] Heartbeat: respond to server `ping` with `pong` within 5s
- [ ] Types live in `frontend/lib/ws-types.ts`, hand-written to match
      architecture §4.2 (no codegen this story)
- [ ] No `any`
- [ ] `pnpm typecheck / lint / build` pass

## Non-goals

- Reactive store (Redux/Zustand) — single hook is enough
- ts-rs codegen (Phase 1)

## Files changed

- `frontend/lib/ws.ts` (new)
- `frontend/lib/ws-types.ts` (new — envelope/command/event shapes)
- `frontend/package.json` (no new runtime deps; native `WebSocket`)

## Spec references

- `/specs/phase-0/architecture.md` §4.2 envelope protocol

## Commit message

```
feat(phase-0): story 0.20 - WebSocket client + reconnection

- useAgentSocket hook: typed envelope client, exponential backoff
  reconnect (1s→30s cap), event ring buffer (1000), per-command
  ack promise routing, ping/pong heartbeat
- ws-types.ts mirrors architecture §4.2 envelope shapes (hand-written
  Phase 0; ts-rs codegen deferred to Phase 1)
- pnpm typecheck/lint/build pass

refs: /specs/phase-0/stories/story-0.20.md
```
