# Story 4.3 — CuratorWorker production runtime + main.rs wiring

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 4.2
> **Phase**: 4
> **Type**: backend+test

---

## Goal

Ship production `CuratorWorker` with interval/backlog triggers, cycle orchestration, and server
boot wiring in `main.rs`.

## Acceptance criteria

- [ ] Production `CuratorWorker` implementation exists (not test-only).
- [ ] Dual trigger model (interval + backlog threshold) implemented.
- [ ] Runtime is wired from `seasoned-hand-server/src/main.rs`.
- [ ] Curator remains async and non-blocking to verifier PASS path.
- [ ] Integration test runs end-to-end cycle with stub dependencies.

## Non-goals

- Consolidation policy details.

---

## Implementation steps

1. Implement `CuratorWorker::run/run_once` and trigger arbitration.
2. Add bootstrap wiring and cancellation handling in `main.rs`.
3. Add integration test for cycle start->complete telemetry.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.1, F-4.2, NFR-4.1, NFR-4.2
- architecture: §2.1, §4.1, §6.5, §7
- debt closed: —
