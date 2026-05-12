# Story 1.19 — Frontend: 3-track BrowserTab (A live noVNC, B DOM text, C screenshot strip)

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 1.16 (backend emits Track B/C events)
> **Phase**: 1
> **Type**: frontend
> **Reads first**: `/specs/phase-1/architecture.md` §2.7 (3-track
> table), §3.4 (`browser_track_c` payload), `/specs/phase-0/stories/story-0.24.md`
> (current noVNC iframe BrowserTab), `/specs/phase-1/DEBT.md` #9
> (no automated FE tests in Phase 1).

---

## Goal

Replace the Phase 0 noVNC-only BrowserTab with a 3-track view: live
noVNC iframe (Track A), scrollable DOM-text pane (Track B), horizontal
screenshot strip (Track C). Clicking a thumbnail in Track C opens a
fullsize overlay. Pure frontend; backend emits the events already.

## Acceptance criteria

- [ ] `frontend/src/panels/AgentComputer/BrowserTab.tsx` is restructured
      into three sub-components rendered in a vertical stack:
      - **Track A**: existing noVNC iframe at top, ≥ 60 % of vertical
        space.
      - **Track B**: scrollable DOM-text pane at bottom-left, ≈ 25 %
        height, monospace font, shows the latest `dom_text_ref`
        resolved via the existing `/v1/workspace/:session_id/*path`
        route (or inline if the Observation carried inline bytes).
      - **Track C**: horizontal screenshot strip at bottom-right (or
        below Track B on narrow viewports), each PNG rendered as a
        clickable thumbnail (max-height 80 px), newest on the right.
- [ ] Track B and Track C subscribe to WS events:
      - Track B updates when an Observation arrives whose tool name
        starts with `browser_`. It pulls `Observation.dom_text_ref`;
        if `FileRef`, fetches the bytes; if `Inline`, uses them
        directly.
      - Track C appends a thumbnail when a `Misc{kind:"browser_track_c"}`
        event arrives, sourcing the image from
        `/v1/workspace/<session_id>/.tracks/<call_id>.png`.
- [ ] Clicking a Track C thumbnail opens a fullsize modal/overlay
      (existing Phase 0 modal primitive — reuse). ESC closes.
- [ ] Track C strip holds at most 100 thumbnails in DOM at once;
      older are unmounted (no infinite scroll history). When the
      backlog exceeds 100, the user sees a "older screenshots hidden"
      label and can scroll-left to load up to 50 more via the
      `/v1/workspace/<session_id>/.tracks/` directory listing
      (frontend lists the directory via the existing static-file
      route).
- [ ] Failure states:
      - Image fetch failure → render a placeholder thumbnail with a
        broken-image glyph; no console error.
      - `browser_track_c_skipped` Misc events render a small grey
        "skipped: <reason>" tile in chronological position (so the
        user knows the agent acted but capture failed).
- [ ] No new frontend dependencies. Tailwind utility classes only.
- [ ] Manual smoke documented; automated tests explicitly deferred
      (phase-1/DEBT.md #9).

## Non-goals

- Pagination beyond the +50 load-older — Phase 4 if needed.
- Diff/overlay of consecutive screenshots — Phase 4.
- Track B search/highlight — out of scope (use browser Find).
- Capturing thumbnails to a separate gallery view — strip view only.

## Implementation steps

### 1. BrowserTab layout

```tsx
// frontend/src/panels/AgentComputer/BrowserTab.tsx
export function BrowserTab({ sessionId, wsEvents, novncUrl }: Props) {
  return (
    <div className="flex h-full flex-col">
      <div className="flex-[3] border-b">
        <NovncIframe url={novncUrl} />
      </div>
      <div className="flex flex-[1] divide-x">
        <div className="flex-1 overflow-y-auto p-2">
          <DomTextPane sessionId={sessionId} wsEvents={wsEvents} />
        </div>
        <div className="flex-1 overflow-x-auto p-2">
          <ScreenshotStrip sessionId={sessionId} wsEvents={wsEvents} />
        </div>
      </div>
    </div>
  );
}
```

### 2. DomTextPane

```tsx
function DomTextPane({ sessionId, wsEvents }: Props) {
  const [text, setText] = useState<string>("");
  useEffect(() => {
    return wsEvents.on(async ev => {
      if (ev.payload.kind !== "Observation") return;
      if (!ev.payload.tool?.startsWith("browser_")) return;
      const ref = ev.payload.dom_text_ref;
      if (!ref) return;
      if (ref.kind === "inline") { setText(new TextDecoder().decode(ref.bytes)); return; }
      const r = await fetch(`/v1/workspace/${sessionId}/${ref.path.replace(/^\//, "")}`);
      setText(await r.text());
    });
  }, [sessionId, wsEvents]);
  return <pre className="font-mono text-xs whitespace-pre-wrap">{text || "(no DOM snapshot yet)"}</pre>;
}
```

### 3. ScreenshotStrip

```tsx
const MAX_VISIBLE = 100;

function ScreenshotStrip({ sessionId, wsEvents }: Props) {
  const [thumbs, setThumbs] = useState<Thumb[]>([]);
  const [openIdx, setOpenIdx] = useState<number | null>(null);

  useEffect(() => {
    return wsEvents.on(ev => {
      if (ev.payload.kind !== "Misc") return;
      const k = (ev.payload as any).kind_tag;
      if (k === "browser_track_c") {
        const { call_id, file_ref } = ev.payload as any;
        setThumbs(t => [
          ...t.slice(-MAX_VISIBLE + 1),
          { call_id, url: `/v1/workspace/${sessionId}/${file_ref.path.replace(/^\//, "")}`, kind: "ok" }
        ]);
      } else if (k === "browser_track_c_skipped") {
        const { call_id, reason } = ev.payload as any;
        setThumbs(t => [
          ...t.slice(-MAX_VISIBLE + 1),
          { call_id, reason, kind: "skipped" }
        ]);
      }
    });
  }, [sessionId, wsEvents]);

  return (
    <>
      <div className="flex h-20 gap-1">
        {thumbs.map((t, i) => t.kind === "ok"
          ? <img key={t.call_id} src={t.url} className="h-full cursor-pointer rounded"
                 onClick={() => setOpenIdx(i)} onError={(e) => markBroken(e)} />
          : <div key={t.call_id} className="flex h-full w-20 items-center justify-center
                                              rounded bg-muted text-[10px] text-muted-foreground">
              skipped: {t.reason}
            </div>
        )}
      </div>
      {openIdx !== null && <Lightbox src={thumbs[openIdx].url} onClose={() => setOpenIdx(null)} />}
    </>
  );
}
```

### 4. Lightbox

Reuse the existing Phase 0 modal primitive (or a tiny new `Lightbox`
component — 20 lines). ESC closes; click backdrop closes.

### 5. Older-screenshots load

A small "load older" button at the left edge of the strip when
`thumbs.length === MAX_VISIBLE` and the backlog hasn't been loaded.
Fetches `GET /v1/workspace/<session_id>/.tracks/` (directory listing —
Phase 0 already supports this via the workspace proxy route) and
hydrates up to 50 older thumbnails by reading metadata from filenames
(`<call_id>.png`).

### 6. Manual-smoke documentation

Add a `## Manual smoke` section to this story consistent with story
1.18 — no automated FE tests in Phase 1.

---

## Verification

```bash
pnpm typecheck
pnpm lint
pnpm build
./scripts/spec-check.sh
```

### Manual smoke

1. Submit a task that browses: "Find the GitHub stars of
   FoundationAgents/OpenManus".
2. The BrowserTab top shows the live noVNC view.
3. As each `browser_*` tool call completes, the DOM-text pane updates
   and a thumbnail appears in the strip.
4. Clicking a thumbnail opens the fullsize PNG; ESC closes.
5. Simulate a screenshot failure (stop the sandbox screenshot service
   mid-task) — a "skipped: <reason>" tile appears instead of an
   image.

---

## Files changed

- `frontend/src/panels/AgentComputer/BrowserTab.tsx` (modify — new layout)
- `frontend/src/panels/AgentComputer/NovncIframe.tsx` (extract from
  Phase 0 BrowserTab, no behavior change)
- `frontend/src/panels/AgentComputer/DomTextPane.tsx` (new)
- `frontend/src/panels/AgentComputer/ScreenshotStrip.tsx` (new)
- `frontend/src/panels/AgentComputer/Lightbox.tsx` (new — small,
  if no existing modal primitive)
- `frontend/src/lib/payload.ts` (modify — `Observation.dom_text_ref`
  type, `BrowserTrackCPayload` type)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.7 (table verbatim), §3.4 (Misc
  payload), §12 q7 (full-resolution; retention deferred).
- `/specs/phase-1/DEBT.md` #8 (retention), #9 (FE tests deferred).

---

## Commit message

```
feat(phase-1): story 1.19 - frontend 3-track BrowserTab

- BrowserTab restructured into vertical stack: Track A (noVNC iframe
  ≥60% height), Track B (DOM-text pane, monospace), Track C (horizontal
  screenshot strip, newest right)
- Track B subscribes to Observation events from browser_* tools,
  resolves Observation.dom_text_ref (inline → bytes; FileRef → fetch
  via /v1/workspace/:session_id/*) and renders in a <pre>
- Track C consumes Misc{kind:"browser_track_c"} events, appending
  thumbnails up to MAX_VISIBLE=100; older auto-unmount; "load older"
  rehydrates up to 50 more from the workspace directory listing.
  browser_track_c_skipped events render a "skipped: <reason>" tile
- Click thumbnail → Lightbox fullsize; ESC / backdrop close
- No new frontend deps; pure React + existing Tailwind tokens
- Manual smoke per phase-1/DEBT.md #9 (FE automated tests deferred)

refs: /specs/phase-1/stories/story-1.19.md
```

---

## Notes for next story (1.20)

All Phase 1 surface area is in place. Story 1.20 is the closing E2E +
retrospective: GAIA-Level-1 fixture, 50-step synthetic task, live-LLM
`workflow_dispatch` smoke, DEBT audit, retrospective doc.
