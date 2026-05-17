# Story 3.13 — Regression: production matcher smoke

> **Status**: done
> **Estimated**: 1.5 hours
> **Dependencies**: 3.5
> **Phase**: 3
> **Type**: test

---

## Goal

Add `phase3_production_matcher_smoke` to ensure FTS5 production matcher returns expected
ranked top-3 on representative seeded data.

## Acceptance criteria

- [ ] `phase3_production_matcher_smoke` test exists and passes.
- [ ] Seeds known playbooks and asserts expected top-3 set/order.
- [ ] Asserts project-scope filtering and archived-row exclusion.

## Non-goals

- Benchmark warm-gate acceptance.

---

## Implementation steps

1. Seed deterministic playbook/task rows.
2. Query production matcher with representative prompts.
3. Assert candidate set, order, and filter invariants.

---

## Verification

```bash
cargo test phase3_production_matcher_smoke
```

---

## Refs

- requirements: F-3.5, F-3.11
- architecture: §11
- debt context: #76, #77
