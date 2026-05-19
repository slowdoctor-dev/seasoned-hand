# Story 4.16 — NFR-4.7 false-positive audit harness

> **Status**: done
> **Estimated**: 2.5 hours
> **Dependencies**: 4.5, 4.13
> **Phase**: 4
> **Type**: test

---

## Goal

Build audit harness for auto-archive and auto-merge false positives with required sample floors.

## Acceptance criteria

- [ ] Harness supports N>=100 archive and N>=100 merge decisions.
- [ ] Harness supports 3 corpus shapes (small/medium/large).
- [ ] Report includes measured false-positive rates and threshold checks.

## Non-goals

- Policy retuning automation.

---

## Implementation steps

1. Build fixture sets for three corpus sizes.
2. Add audit runner and summary output.
3. Add CI gate target for NFR-4.7 pass/fail.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: NFR-4.7
- architecture: §11.3, §12.2
- debt closed: #90 (verification close)
