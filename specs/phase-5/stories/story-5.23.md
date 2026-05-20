# Story 5.23 — Per-crate dependency justification (closes DEBT #97)

> **Status**: done
> **Estimated**: 1 hour
> **Dependencies**: 5.1
> **Phase**: 5
> **Type**: docs

---

## Goal

Per F-5.20 + DEBT #97: every net-new dependency added during Phase 5 implementation must have
a per-crate justification entry in `specs/01-architecture/ARCHITECTURE.md` §1 addendum table.

## Acceptance criteria

- [ ] Update ARCH §1 addendum table with any new Phase 5 dependencies (likely none if Phase 5
      sticks to existing crates; this story closes the discipline either way).
- [ ] CI hook (in `scripts/spec-check.sh`) asserts: every dep in `Cargo.toml` workspace root
      that didn't exist at Phase 4 baseline has an addendum entry.
- [ ] If zero new deps land in Phase 5, the story still closes by adding the spec-check rule.

## Verification

```bash
bash scripts/spec-check.sh   # 9/9 after this story
```

## Refs

- requirements: F-5.20
- debt closed: #97 (close)
