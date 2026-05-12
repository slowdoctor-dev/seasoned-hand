# Story 0.21 — Chat component (notify rendering + input)

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: 0.19 (layout), 0.20 (WS client)
> **Phase**: 0
> **Type**: frontend
> **Reads first**: `/specs/phase-0/architecture.md` §1, §3.4 (Message event shape)

## Goal

Replace the center-panel placeholder with a real Chat that:
1. Renders the live event stream as a chat thread (Message,
   Observation, and Misc events, in time order)
2. Has a single text input + Submit button that posts a `task_create`
   command for a new session, or a `user_response` if the active
   session is SUSPENDED with an open `message_ask_user`

## Acceptance criteria

- [ ] `<Chat sessionId={id|null}/>` reads from `useAgentSocket`'s
      `events` array filtered to `sessionId`
- [ ] If `sessionId === null` (no session yet): the input creates a
      session via `task_create`; the returned `ack.session_id`
      becomes the active session
- [ ] If the latest Message in the session has `ui: "ask"` and no
      following `user_response` event: the input is labeled
      "Reply..." and submitting sends `user_response { session_id,
      in_reply_to_call_id: <call_id from ask>, content }`
- [ ] Otherwise the input is disabled while the session is RUNNING
- [ ] Renders 3 event types into 3 row variants:
      - `Message` (assistant): bubble, gray bg
      - `Message` (user): bubble, blue bg, right-aligned
      - `Observation`: compact, monospace `tool_name → ok|err`
      - `Misc`: small italic, gray (truncated to first 80 chars)
- [ ] Auto-scrolls to bottom when new events arrive (unless the user
      has manually scrolled up)
- [ ] `pnpm typecheck / lint / build` pass

## Non-goals

- Markdown rendering in messages (Phase 1)
- Image/file attachments (Phase 1+)
- Message editing or threading (Phase 1+)

## Files changed

- `frontend/components/chat.tsx` (new)
- `frontend/components/panels/chat-placeholder.tsx` → real chat wrapped
- `frontend/lib/chat-state.ts` (new — derives ask/idle state from event list)

## Spec references

- `/specs/phase-0/architecture.md` §1, §3.4

## Commit message

```
feat(phase-0): story 0.21 - Chat component

- Chat renders Message/Observation/Misc events as 4 row variants
- Input: task_create when no session, user_response when SUSPENDED
  on a message_ask_user, disabled otherwise
- Auto-scroll-to-bottom; respects manual scroll-up
- pnpm typecheck/lint/build pass

refs: /specs/phase-0/stories/story-0.21.md
```
