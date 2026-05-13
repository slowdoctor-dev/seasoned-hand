# Story 2.23 — Frontend: Briefing card + confirm/edit/cancel UI

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 2.8 (backend Initializer confirm gate)
> **Phase**: 2
> **Type**: frontend
> **Reads first**: `/specs/phase-2/architecture.md` §2.2 (Briefing protocol)

---

## Goal

The user-facing half of the briefing protocol. Chat panel intercepts
`Briefing` ServerEvents and renders a card with Confirm / Edit /
Cancel actions. The card stays interactive until one of the actions
fires OR auto-confirm timeout passes (server-side).

## Acceptance criteria

- [ ] `frontend/components/chat/briefing-card.tsx` (new) renders:
      - Goal (1-3 sentence header)
      - Phases (numbered list with titles)
      - Success criteria (bulleted list)
      - Expected deliverables (filename + format chip per item)
      - Three buttons: **Confirm** (primary), **Edit** (secondary
        — opens an inline editor), **Cancel** (destructive).
- [ ] `Chat.tsx` event-row renderer detects `payload.kind ===
      "Briefing"` and substitutes the BriefingCard inline (replacing
      the regular Message branch).
- [ ] On **Confirm**: sends WS `{ cmd: "briefing_confirm", task_id,
      in_reply_to_call_id: briefing_call_id, action: "confirm" }`.
      Card disables all three buttons + shows "Confirmed at HH:MM:SS".
- [ ] On **Cancel**: sends `{ cmd: "briefing_confirm", action:
      "cancel" }`. Card replaced with "Cancelled."
- [ ] On **Edit**: card swaps into edit mode with a JSON textarea
      pre-filled with the current `Brief`. "Save" button sends
      `{ cmd: "briefing_confirm", action: "edit", edits: <parsed_json> }`.
      "Discard" reverts to the read-only card. JSON parse error
      blocks Save and shows inline error.
- [ ] After server processes Edit, a NEW `Briefing` event arrives
      with a new `briefing_call_id`. The OLD card stays visible but
      marked "Superseded" (greyed); the NEW card is the active one.
- [ ] After auto-confirm timeout (server-driven; Phase 2 default 5
      min), the server emits `Misc{kind:"briefing_auto_confirmed"}`.
      The card detects this Misc, marks itself "Auto-confirmed."
- [ ] `pnpm typecheck` + `pnpm lint` + `pnpm build` clean.

## Non-goals

- In-place per-field editing (Phase 2 ships JSON textarea only;
  field-level editor is Phase 4+ UX).
- Validation feedback for the edits (server-side validation from
  story 2.7 fires; UI just surfaces the error message).
- Persisting card collapsed/expanded state across reloads (acceptable
  to start collapsed each session).

---

## Implementation steps

### 1. BriefingCard component

```tsx
type BriefingPayload = {
  kind: "Briefing";
  briefing_call_id: string;
  goal: string;
  phases: Array<{ id: number; title: string; capabilities?: string[] }>;
  success_criteria: string[];
  expected_deliverables: Array<{ filename: string; format: string; description?: string }>;
};

export function BriefingCard({
  briefing,
  taskId,
  superseded,
  resolution,
  send,
}: Props) {
  const [mode, setMode] = useState<"view" | "edit">("view");
  const [editJson, setEditJson] = useState(JSON.stringify(briefing, null, 2));
  const [error, setError] = useState<string | null>(null);
  // ... render ...
}
```

### 2. Chat.tsx integration

```tsx
function EventRow({ event, taskId, send }: Props) {
  const p = event.payload;
  if (p.kind === "Briefing") {
    return <BriefingCard briefing={p} taskId={taskId} send={send} ... />;
  }
  // ... existing branches ...
}
```

### 3. Resolution tracking

The Chat component keeps a `Map<briefing_call_id, "confirmed" |
"cancelled" | "auto_confirmed" | "superseded">` derived from events
seen so far. Each new `briefing_confirm` cmd → optimistically updates
the map; the server's response Misc event (or new Briefing event for
edit) confirms.

### 4. Manual smoke

After `pnpm dev`:
- Submit `task_create` from Chat → Briefing card appears
- Click Edit → JSON textarea opens → modify a phase → Save → new
  Briefing event arrives, old card greyed out
- On a separate task: don't click anything for 5 min → card transitions
  to "Auto-confirmed"

---

## Verification

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend lint
pnpm --dir frontend build
./scripts/spec-check.sh
```

---

## Files changed

- `frontend/components/chat/briefing-card.tsx` (new)
- `frontend/components/chat.tsx` (modify — intercept Briefing payload)
- `frontend/lib/ws-types.ts` (modify — BriefingPayload type, additions
  to ClientCommand: `briefing_confirm`)

---

## Spec references

- `/specs/phase-2/architecture.md` §2.2 (Briefing protocol), §4
  (WS additions)

---

## Commit message

```
feat(phase-2): story 2.23 - Frontend Briefing card + confirm/edit/cancel UI

The user-facing half of story 2.8's confirm gate.

- BriefingCard intercepts payload.kind === "Briefing" events in the
  Chat event-row mapper. Shows goal / phases / criteria /
  deliverables + three buttons.
- Confirm / Cancel send { cmd: "briefing_confirm", action } WS cmds.
- Edit opens an inline JSON textarea; Save sends { action: "edit",
  edits: <json> }. Server re-emits a new Briefing event with a new
  briefing_call_id; old card greyed as "Superseded".
- Auto-confirm Misc transitions the card to "Auto-confirmed" state.
- pnpm typecheck + lint + build clean.

refs: /specs/phase-2/stories/story-2.23.md
```

---

## Notes for next story (2.24)

Three new FE surfaces are in (ProjectList + Deliverables + Decisions
+ BriefingCard = the four Phase-2 frontend additions). 2.24 closes
DEBT #9 by bootstrapping Playwright + writing smoke coverage for all
four.
