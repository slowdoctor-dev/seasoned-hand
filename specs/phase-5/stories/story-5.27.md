# Story 5.27 — phase5_rbac_matrix_harness (NFR-5.2)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 5.5, 5.6
> **Phase**: 5
> **Type**: test

---

## Goal

Verify every cell of the §4.3 RBAC matrix returns the right decision. Verify project-role
override precedence. Verify p95/p99 auth-check latency NFR-5.2.

## Acceptance criteria

- [ ] `phase5_rbac_matrix_harness` exists and exercises 8 actions × 3 roles = 24 cells.
- [ ] Project-override scenarios: user with org-`viewer` + project-`user` override gets `user`
      decisions on that project.
- [ ] p95 ≤ 10 ms, p99 ≤ 25 ms on baseline runner (NFR-5.2).
- [ ] CI budget < 3 min.

## Verification

```bash
cargo test -p seasoned-hand-core phase5_rbac_matrix_harness
```

## Refs

- requirements: NFR-5.2, F-5.5
- architecture: §15 harness 2
