# Phase 3 — Technical Debt Ledger

> Append-only list of shortcuts, stubs, simplifications, and deferred
> work introduced during Phase 3. Same discipline as Phase 0 / Phase 1
> / Phase 2 DEBT.md.
>
> **Status (pre-kickoff, 2026-05-16)**: empty header. Phase 3 has not
> yet been seeded by the BMAD Architect pass on
> `/specs/phase-3/architecture.md`. Seed items will land here once
> the architecture doc is written and reviewed.
>
> Pay-down inheritance from earlier phases (informational only — do
> NOT duplicate entries here, see the source ledgers):
>
> - Phase 2 DEBT #5 — Provenance manifest size budget (Curator may
>   compress old manifests at Phase 3+)
> - Phase 2 DEBT #6 — Skill / playbook tables empty (Phase 3 fills them)
> - Phase 2 DEBT #7 — Verifier rollback default still opt-in
>   (re-evaluate with Phase 3 verdict data)
> - Phase 2 DEBT #28 — Replay cost baseline resets to zero on rebuild
>   (Phase 3 may emit periodic `cost_snapshot` Misc events)
> - Phase 2 DEBT #31 (rough edge 3) — BriefingCard server-side
>   validation error UX
> - Phase 2 DEBT #52 — `lib.rs` 2879-line split (Phase 3 warm-up)
> - Phase 2 DEBT #58 (remainder) — `pub` → `pub(crate)` shrinkage
> - Phase 2 DEBT #60 — Phase 1 large-file split set
> - Phase 2 DEBT #61 — `EventType::Knowledge/Datasource/Skill` emit
>   wiring (Phase 3 Curator)
> - Phase 2 DEBT #62 — `spec-check.sh` phase-version gate
> - Phase 2 DEBT #63 — Frontend `pnpm test` stub
> - Phase 1 DEBT #3 — Verifier rollback default (see Phase 2 DEBT #7)
>
> Closed by the Codex follow-up sequence (2026-05-17) before Phase 3
> kickoff (informational — these no longer carry to Phase 3):
>
> - Phase 2 DEBT #65 — Phase 1 verifier + checkpoint route loopback
> - Phase 2 DEBT #66 — /ws loopback gate
> - Phase 2 DEBT #67 — checkpoint rollback state coherence
> - Phase 2 DEBT #68 — plan phase/title caps
> - Phase 2 DEBT #69 — loopback regression test sweep
> - Phase 2 DEBT #70 — channel introspection routes loopback
> - Phase 2 DEBT #71 — Track C screenshot byte cap

---

## Seed (TBD)

_To be populated by the BMAD Architect pass on
`/specs/phase-3/architecture.md`._

---

## Categories quick-reference (same as Phase 0 / Phase 1 / Phase 2)

| Severity | Meaning |
|---|---|
| **H** | Blocks the next phase's goals if not addressed |
| **M** | Will bite at scale or in a year, manageable today |
| **L** | Documentation / minor friction / one-line fix later |
