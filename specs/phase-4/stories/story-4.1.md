# Story 4.1 — Phase 4 scaffolds (requirements + architecture + OPEN_QUESTIONS + DEBT + REVIEW + story map)

> **Status**: done (retroactive; shipped by Analyst + Architect + REVIEW passes)
> **Estimated**: 0.5 hours
> **Dependencies**: —
> **Phase**: 4
> **Type**: doc

---

## Goal

Declare the Phase 4 planning foundation as ready and publish the concrete story map in
`requirements.md` §4.

## Acceptance criteria

- [ ] `/specs/phase-4/requirements.md` exists and is the phase contract.
- [ ] `/specs/phase-4/architecture.md` exists and is the technical design baseline.
- [ ] `/specs/phase-4/OPEN_QUESTIONS.md` has 20/20 resolved footers.
- [ ] `/specs/phase-4/DEBT.md` exists with inherited + architect/review seed entries.
- [ ] `/specs/phase-4/REVIEW.md` exists with analyst and architect hardening history.
- [ ] `/specs/phase-4/requirements.md` §4 contains the full story breakdown table.

## Non-goals

- Any runtime implementation.

---

## Implementation steps

1. Mark this story done retroactively.
2. Replace requirements §4 placeholder with concrete story table (4.1-4.22).

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.26
- architecture: §13
