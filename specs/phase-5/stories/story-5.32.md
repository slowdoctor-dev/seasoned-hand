# Story 5.32 — phase5_team_simulation_benchmark (5-actor, ≤60 min CI budget)

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: 5.26-5.31
> **Phase**: 5
> **Type**: test+benchmark

---

## Goal

The Phase 5 headline acceptance harness, analogous to Phase 4's `phase4_warm_full_loop_benchmark`
(story 4.21). Simulates a 5-person team (1 admin, 3 users, 1 viewer) using one instance
concurrently with task hand-offs, shared SOP/playbook usage, and per-user cost ledger
reconciliation.

## Acceptance criteria

- [ ] `phase5_team_simulation_benchmark` test exists.
- [ ] Fixture: 5 user identities, 1 org, 3 projects, 50+ tasks across the project set with
      concurrent agent loops via test stubs (no real LLM, deterministic replay).
- [ ] Drives: task creation by 3 users, hand-offs across pairs, SOP shares (some accepted, some
      rejected per RBAC matrix), playbook auto-share via curator high-confidence path, audit
      log reads, per-user cost rollup.
- [ ] Asserts: zero cross-tenant leakage, all RBAC denials emit correctly, audit_log captures
      every mutating op, per-user cost ledger reconciles within +/-0.5%.
- [ ] Wall-clock CI budget ≤ 60 min (per acceptance §1).

## Verification

```bash
cargo test -p seasoned-hand-core phase5_team_simulation_benchmark
```

## Refs

- requirements: F-5.24, all NFRs
- architecture: §15 harness 1 (composed acceptance benchmark)
