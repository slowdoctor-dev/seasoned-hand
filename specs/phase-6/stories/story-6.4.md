# Story 6.4 — Full-fidelity port of remaining React surface + ack handling

> **Status**: in-progress

Bring the Dioxus UI to parity with the Next.js app beyond the foundation.

## Acceptance criteria

- [x] **Ack handling (session capture)**: the ws.rs coroutine correlates a
      `task_create` ack by `ref` and writes the assigned `session_id` into the
      shared selection signal, so the UI subscribes to the new run. *(Remaining:
      a general per-command ack-await Future API for arbitrary commands.)*
- [x] **Briefing card** (`briefing-card.tsx`): renders `Misc{kind_tag:"briefing"}`
      events with goal/phases/criteria/deliverables; **confirm / edit / cancel**
      via `briefing_confirm` (keyed by task_id). Edit opens a JSON textarea and
      sends `action:"edit"` with parsed edits. Resolution taxonomy (issue #3):
      cards derive superseded (a later briefing for the same task) and
      auto-confirmed (`Misc{kind:"briefing_auto_confirmed"}`) states from the
      event stream, alongside the local resolved flag.
- [x] **AgentComputer — Deliverables tab**: lists `/v1/tasks/:id/deliverables`.
- [x] **AgentComputer — Files tab**: **recursive** workspace tree — dirs expand
      lazily (`/v1/workspace/:id/{path}`), clicking a file loads it into the
      Editor tab (Monaco, re-mounts per path) via panel-shared state.
- [x] **AgentComputer — Verifier tab**: lists
      `/v1/sessions/:id/verifications` with pass/fail verdicts + reasons.
- [x] **AgentComputer — Decisions tab**: filters the live event stream for
      `Misc{kind_tag:"decision"}` events (source + reason), no endpoint.
- [x] **AgentComputer — browser-track visualizers** (issue #3,
      `components/browser_track.rs`): Browser tab splits into noVNC (top) +
      Track B DOM-text pane / Track C screenshot strip (bottom). The strip caps
      at 100 live thumbnails with a "load older" backfill from
      `/workspace/.tracks/`, renders skip markers, and opens a lightbox on
      click. Screenshots fetch through the auth-gated workspace proxy with the
      bearer token and render as `data:` URLs (a plain `<img src>` cannot carry
      the ADR-018 session token). *(Rendering against real observation events
      from a live Docker task remains to be eyeballed — tracked with story
      6.2's live-session gate.)*
- [x] **Per-session event index** for evidence-chip O(1) lookup (parity with
      `HomeShell`'s `eventIndex`): the Verifier tab builds it and verdict rows
      expand to evidence chips + the optional `suggested_plan_update`.
- [x] Reactive interop updates: the interop effects are `use_reactive` on
      their props; the shims are idempotent per mount id (Monaco swaps the
      model value/language in place, xterm/noVNC dispose + re-attach) and
      `use_drop` → `__disposeInterop(id)` tears down on unmount, so tab
      switches don't leak instances.
- [ ] Visual + behavioural parity reviewed against the removed Next.js app
      (needs Docker — story 6.2's live-session gate).
