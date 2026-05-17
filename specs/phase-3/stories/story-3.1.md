# Story 3.1 — Phase 3 scaffolds (requirements + architecture + DEBT + story map)

> **Status**: done (retroactive; shipped by Analyst + Architect + PM passes)
> **Estimated**: 0.5 hours
> **Dependencies**: —
> **Phase**: 3
> **Type**: doc

---

## Goal

Declare the Phase 3 planning foundation as ready: requirements contract, architecture
spec, seeded debt ledger, and the PM story breakdown table.

## Acceptance criteria

- [ ] `/specs/phase-3/requirements.md` exists and is the phase contract.
- [ ] `/specs/phase-3/architecture.md` exists and is the technical design baseline.
- [ ] `/specs/phase-3/DEBT.md` exists with inherited + seeded entries.
- [ ] `/specs/phase-3/requirements.md` includes the §4 story breakdown table.
- [ ] `bash scripts/spec-check.sh` passes.

## Non-goals

- Implementing any runtime code.
- Shipping V010 migration or CLI/runtime surfaces.

---

## Implementation steps

1. Mark this story as done retroactively.
2. Publish the complete story breakdown table in `requirements.md`.

---

## Verification

```bash
test -f specs/phase-3/requirements.md
test -f specs/phase-3/architecture.md
test -f specs/phase-3/DEBT.md
bash scripts/spec-check.sh
```

---

## Refs

- requirements: phase contract baseline
- architecture: full phase design baseline
- debt: #72-#79 seed context
