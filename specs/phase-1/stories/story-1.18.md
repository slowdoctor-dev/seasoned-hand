# Story 1.18 — Frontend: narration lane + Verifier verdict pane

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 1.9 (HTTP routes `GET /v1/sessions/:id/verifications`
> and `GET /v1/verifications/:id`), 1.10 (verdict events flow over WS;
> Gate emits `verifier_verdict` Misc), 1.15 (narration events flow over WS)
> **Phase**: 1
> **Type**: frontend
> **Reads first**: `/specs/phase-1/architecture.md` §1 ("Frontend"
> diagram block), §4.2 (WS envelope additions), §12 q1 (verdict pane
> lazy-load decision), `/specs/phase-0/stories/story-0.21.md` (Chat
> component), `/specs/phase-0/stories/story-0.23.md` (AgentComputer tabs).

---

## Goal

Two frontend additions to the Phase 0 panels:

1. **Narration lane in Chat** — render `Message{ui:"narrate"}` events in
   a distinct lighter-weight style, threaded with regular messages.
2. **Verifier verdict pane** — new tab in AgentComputer that lists
   `Misc{kind:"verifier_verdict"}` events, with lazy-loaded evidence
   detail when the user expands a row.

Backend already emits the events; this story is pure frontend.

## Acceptance criteria

- [ ] `frontend/src/panels/Chat.tsx` (or its current path) gains a
      narration row renderer:
      - Events filtered by `payload.kind === "Message" && payload.ui === "narrate"`.
      - Rendered inline among user/assistant messages, with class
        `text-xs text-muted opacity-70 italic`.
      - Prefixed with an em-dash glyph (`— `).
      - Click → no-op (Phase 1 narration is non-interactive).
- [ ] Narration events are **always** rendered immediately on WS arrival;
      no pagination or batching.
- [ ] `frontend/src/panels/AgentComputer/VerifierTab.tsx` is a new tab:
      - Lists verifier-verdict events newest-first.
      - Each row: pass/fail badge, reason (one line, truncated to 80
        chars), trigger kind, model id, created_at.
      - Click row → expands to show `evidence_event_ids` as clickable
        chips + the `suggested_plan_update` JSON if present. Chip
        resolution uses the **client-side event index already
        maintained by the Chat panel for the session** (Phase 0 keeps
        a `Map<event_id, Event>` indexed by id; the chip looks up
        synchronously). If the event is outside the loaded window
        (older than the WS replay buffer), the chip renders as
        `"#<id> (older than loaded window)"` with no fetch — no new
        backend route required. This matches architecture §12 q1
        (lazy / no prefetch).
      - Empty state: "No verifier runs yet for this session."
- [ ] AgentComputer tab strip from Phase 0 (story 0.23) gains
      "Verifier" as a new tab (right of "Editor" or wherever fits the
      existing order). Persisted as the active tab via existing tab
      state.
- [ ] New tab is hydrated by polling
      `GET /v1/sessions/:id/verifications?limit=50` on mount + every WS
      `Misc{kind:"verifier_verdict"}` event (push-driven refresh).
- [ ] No new frontend dependencies. Pure React + Tailwind, matching
      Phase 0 style.
- [ ] Manual smoke noted in `Files changed` — automated frontend tests
      explicitly deferred to Phase 2 per phase-1/DEBT.md #9.

## Non-goals

- 3-track BrowserTab — story 1.19.
- Real-time animation / typing-effect on narration — render whole
  string at once.
- Pretty-printing or syntax highlighting of `suggested_plan_update`
  JSON — `<pre>` with `JSON.stringify(..., null, 2)` is fine.
- Translating evidence_event_ids into human descriptions on hover —
  click-to-expand is enough for Phase 1.
- A "rerun verifier" button — out of scope; that's a Phase 4
  introspection feature.

## Implementation steps

### 1. Narration renderer

```tsx
// frontend/src/panels/Chat/MessageRow.tsx (or equivalent)
function isNarration(p: Payload): boolean {
  return p.kind === "Message" && (p as MessagePayload).ui === "narrate";
}

function NarrationRow({ p }: { p: MessagePayload }) {
  return (
    <div className="px-3 py-1 text-xs italic opacity-70 text-muted-foreground" aria-label="narration">
      — {p.content}
    </div>
  );
}
```

In the Chat component's event-list mapper, dispatch on the payload's
`ui` field before the regular Message branch:

```tsx
{events.map(ev => {
  if (isNarration(ev.payload)) return <NarrationRow key={ev.id} p={ev.payload}/>;
  if (ev.payload.kind === "Message") return <MessageRow .../>
  // existing branches
})}
```

### 2. VerifierTab

```tsx
// frontend/src/panels/AgentComputer/VerifierTab.tsx
export function VerifierTab({ sessionId, wsEvents }: Props) {
  const [verdicts, setVerdicts] = useState<Verdict[]>([]);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  useEffect(() => {
    fetch(`/v1/sessions/${sessionId}/verifications?limit=50`)
      .then(r => r.json()).then(setVerdicts);
  }, [sessionId]);

  useEffect(() => {
    const unsub = wsEvents.on(ev => {
      if (ev.payload.kind === "Misc" && (ev.payload as any).kind_tag === "verifier_verdict") {
        // Refresh list (or prepend the single new verdict)
        fetch(`/v1/sessions/${sessionId}/verifications?limit=50`)
          .then(r => r.json()).then(setVerdicts);
      }
    });
    return () => unsub();
  }, [wsEvents, sessionId]);

  if (verdicts.length === 0) {
    return <div className="p-4 text-sm text-muted-foreground">No verifier runs yet for this session.</div>;
  }
  return (
    <ul className="divide-y">
      {verdicts.map(v => (
        <VerdictRow key={v.id} v={v}
          expanded={expanded.has(v.id)}
          onToggle={() => toggle(expanded, v.id, setExpanded)} />
      ))}
    </ul>
  );
}
```

`VerdictRow` shows the badge + reason summary; on expand, fetches
`/v1/verifications/:id` for full detail and renders evidence chips +
suggested_plan_update.

### 3. Evidence chip — client-side index lookup

The frontend already keeps a per-session `Map<event_id, Event>` index
populated by both (a) the WS event stream and (b) the initial event-
list fetch on session load (Phase 0 story 0.20 wires this). The
chip looks events up synchronously; no new HTTP route is involved.

```tsx
function EvidenceChip({ eventId, eventIndex }: { eventId: number; eventIndex: Map<number, Event> }) {
  const [open, setOpen] = useState(false);
  const event = eventIndex.get(eventId);
  if (!event) {
    return (
      <span className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground"
            title="Event is older than the currently loaded window">
        #{eventId} (older than loaded window)
      </span>
    );
  }
  return (
    <button className="rounded bg-secondary px-2 py-0.5 text-xs" onClick={() => setOpen(o => !o)}>
      #{eventId}
      {open && <pre className="mt-2 max-w-md text-left">{JSON.stringify(event, null, 2)}</pre>}
    </button>
  );
}
```

`eventIndex` is threaded down from the panel that owns the WS
connection (the same component that feeds `Chat`). No backend
addition. If the team later wants out-of-window resolution, a Phase 2
story can add `GET /v1/events/:id` — explicitly *not* in scope here.

### 4. Tab registration

In the AgentComputer panel's tab list (story 0.23), add `Verifier`
with `<VerifierTab sessionId=... wsEvents=... />`.

### 5. CSS variables

Reuse the existing Phase 0 Tailwind v4 palette tokens — `text-muted-foreground`,
`bg-secondary` — no new tokens needed.

### 6. Manual smoke section

Add a `## Manual smoke` block to this story's verification — Phase 0
shipped the frontend manual-smoke-only and this story preserves that
practice.

---

## Verification

```bash
pnpm typecheck
pnpm lint
pnpm build       # ensures no runtime errors at SSR time
./scripts/spec-check.sh
```

### Manual smoke

1. Start the full stack (`just up`).
2. Submit a task: "Find the GitHub stars of FoundationAgents/OpenManus".
3. Expect narration lines in Chat (e.g. "— Reading the page", "—
   Searching workspace content").
4. After completion, open the Verifier tab → see one `pass` row from
   the TaskComplete trigger.
5. Expand the row → evidence chips visible; clicking one fetches the
   event body.

---

## Files changed

- `frontend/src/panels/Chat/MessageRow.tsx` (modify — narration branch)
- `frontend/src/panels/Chat/NarrationRow.tsx` (new)
- `frontend/src/panels/AgentComputer/VerifierTab.tsx` (new)
- `frontend/src/panels/AgentComputer/VerdictRow.tsx` (new)
- `frontend/src/panels/AgentComputer/EvidenceChip.tsx` (new)
- `frontend/src/panels/AgentComputer/index.tsx` (modify — register tab)
- `frontend/src/lib/payload.ts` (modify — `MessagePayload.ui` type, `Verdict` type)

---

## Spec references

- `/specs/phase-1/architecture.md` §1 (frontend diagram block — narration
  lane, verifier verdict pane), §4.2 (`ui:"narrate"` envelope),
  §12 q1 (lazy evidence load).
- `/specs/phase-1/DEBT.md` #9 (frontend automated tests deferred).

---

## Commit message

```
feat(phase-1): story 1.18 - frontend narration lane + Verifier verdict pane

- Chat panel: Message events with ui:"narrate" render in a lighter
  italic style with em-dash prefix; threaded inline with regular
  messages
- AgentComputer gains a "Verifier" tab listing verifier_verdict
  events newest-first; pass/fail badge, one-line reason, trigger kind,
  model id, created_at
- Row expand lazy-loads /v1/verifications/:id for full detail;
  evidence chips resolve event_ids from the per-session
  Map<event_id, Event> the frontend already maintains (no new
  backend route; out-of-window events render "older than loaded
  window") — architecture §12 q1 lazy decision honored without a
  new fetch path
- Tab hydrates from /v1/sessions/:id/verifications on mount and
  refreshes on every WS Misc{kind:"verifier_verdict"} arrival
- No new frontend dependencies; pure React + existing Tailwind tokens
- Manual smoke verified per phase-1/DEBT.md #9 (automated FE tests
  deferred to Phase 2)

refs: /specs/phase-1/stories/story-1.18.md
```

---

## Notes for next story (1.19)

Two of three frontend lanes are in place. Story 1.19 adds the 3-track
BrowserTab — backend already publishes the events (Track B via
`Observation.dom_text_ref`, Track C via `Misc{kind:"browser_track_c"}`);
the frontend only needs to render them.

---

## Execution notes

**Spec divergence — file layout.** The spec used `frontend/src/panels/`
and `frontend/src/lib/` paths from a sketch; the actual Phase 0 tree
ships at `frontend/components/` and `frontend/lib/`. All new files were
placed in the real tree (`components/agent-computer/verifier-tab.tsx`,
`lib/api.ts` extensions). No `payload.ts` exists — the WS payload type
in `ws-types.ts` is already an open `{ kind; [key: string]: unknown }`
discriminated by `kind`, so the narration / verdict branches cast
inline.

**WS hook lifted to `HomeShell`.** Phase 0's `Chat` owned a private
`useAgentSocket(WS_URL)`. The Verifier tab needs the same WS event
stream to detect new `verifier_verdict` Misc events and trigger a
re-fetch. Lifting the hook into `HomeShell` and passing `events`/`send`
down as props was a few-line change that (a) honors the spec's
"push-driven refresh" without opening a second WebSocket, and (b)
gives the future 1.19 BrowserTab a free seat at the same event stream.
Side benefit: one shared `Map<event_id, ServerEvent>` index now lives
at the root and is passed down to the Verifier tab for synchronous
evidence-chip lookup (architecture §12 q1 "lazy / no prefetch").

**Verifier list refresh is `refreshTick`-based, not optimistic.** The
spec sketch refreshed by re-fetching `/v1/sessions/:id/verifications`
on every WS verifier_verdict arrival. Implementation: when the tab
sees a new (id-keyed) verifier_verdict event, it bumps a `refreshTick`
counter; the fetch effect is keyed on `[sessionId, refreshTick]` so it
re-runs. Keeps the rendering path single-source-of-truth (the HTTP
endpoint) — cheaper to reason about than reconciling a partial WS
payload against the full Verification DTO from the DB. Cost: one HTTP
round-trip per verdict.

**Out-of-window evidence-chip stub matches spec §12 q1 verbatim.** No
new backend route; chips that miss the client-side event index render
as `"#<id> (older than loaded window)"` with no fetch. A Phase 2 story
can add `GET /v1/events/:id` if the team decides lazy single-event
resolution is worth a route.
