# Story 6.4 — Full-fidelity port of remaining React surface + ack handling

> **Status**: in-progress

Bring the Dioxus UI to parity with the Next.js app beyond the foundation.

## Acceptance criteria

- [x] **Ack handling (session capture)**: the ws.rs coroutine correlates a
      `task_create` ack by `ref` and writes the assigned `session_id` into the
      shared selection signal, so the UI subscribes to the new run. *(Remaining:
      a general per-command ack-await Future API for arbitrary commands.)*
- [x] **Briefing card** (`briefing-card.tsx`): renders `Misc{kind_tag:"briefing"}`
      events with goal/phases/criteria/deliverables; **confirm / cancel** via
      `briefing_confirm` (keyed by task_id). *(Remaining: JSON-edit flow + the
      superseded/auto-confirmed resolution taxonomy.)*
- [x] **AgentComputer — Deliverables tab**: lists `/v1/tasks/:id/deliverables`
      for the active task.
- [ ] **AgentComputer — remaining tabs**: verifier, decisions, file-tree,
      screenshot strip, dom-text pane, evidence chips, lightbox.
- [ ] **Per-session event index** for evidence-chip O(1) lookup (parity with
      `HomeShell`'s `eventIndex`) — deferred until the verifier tab consumes it.
- [ ] Reactive interop updates (re-push terminal output / swap editor models),
      not just initial mount.
- [ ] Visual + behavioural parity reviewed against the Next.js app (needs Docker).
