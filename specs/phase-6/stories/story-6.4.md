# Story 6.4 — Full-fidelity port of remaining React surface + ack handling

> **Status**: ready

Bring the Dioxus UI to parity with the Next.js app beyond the foundation.

## Acceptance criteria

- [ ] **Briefing card** (`briefing-card.tsx`): render `Misc{kind:"briefing"}`
      events; confirm / edit / cancel via `briefing_confirm` (keyed by task_id).
- [ ] **AgentComputer tabs**: deliverables, verifier, decisions, file-tree,
      screenshot strip, dom-text pane, evidence chips, lightbox.
- [ ] **Ack handling**: reimplement the `lib/ws.ts` ack-await — correlate server
      `ack` envelopes by `ref` to resolve per-command futures; capture the new
      `session_id` from `task_create` acks and set the active session.
- [ ] **Per-session event index** for evidence-chip O(1) lookup (parity with
      `HomeShell`'s `eventIndex`).
- [ ] Reactive interop updates (re-push terminal output / swap editor models),
      not just initial mount.
- [ ] Visual + behavioural parity reviewed against the Next.js app.
