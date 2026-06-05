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
      sends `action:"edit"` with parsed edits. *(Remaining: the
      superseded/auto-confirmed resolution taxonomy.)*
- [x] **AgentComputer — Deliverables tab**: lists `/v1/tasks/:id/deliverables`.
- [x] **AgentComputer — Files tab**: lists the workspace root
      (`/v1/workspace/:session_id/`). *(Remaining: recursive dir expansion +
      file open into the editor.)*
- [x] **AgentComputer — Verifier tab**: lists
      `/v1/sessions/:id/verifications` with pass/fail verdicts + reasons.
- [ ] **AgentComputer — remaining tabs**: decisions, screenshot strip,
      dom-text pane, evidence chips, lightbox (event-derived browser tracks).
- [ ] **Per-session event index** for evidence-chip O(1) lookup (parity with
      `HomeShell`'s `eventIndex`) — deferred until the verifier tab consumes it.
- [ ] Reactive interop updates (re-push terminal output / swap editor models),
      not just initial mount.
- [ ] Visual + behavioural parity reviewed against the Next.js app (needs Docker).
