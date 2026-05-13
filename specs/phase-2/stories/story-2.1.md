# Story 2.1 — Phase 2 scaffolds (requirements.md + DEBT.md)

> **Status**: done (created alongside the PM session — the BMAD PM persona's first output)
> **Estimated**: 0.5 hours
> **Dependencies**: —
> **Phase**: 2
> **Type**: doc

---

## Goal

Land the two foundation docs every Phase 2 story will reference:
`/specs/phase-2/requirements.md` (the story breakdown + acceptance
criteria + scope boundaries) and `/specs/phase-2/DEBT.md` (the
append-only ledger seeded with the 9 architecture-phase debts).

## Acceptance criteria

- [ ] `/specs/phase-2/requirements.md` exists with §1-§6 (Goals,
      Non-functional, Functional, Story breakdown, Acceptance criteria,
      Deferred). §4 has all 27 stories with `Status: ready` (except
      this one with `Status: done`).
- [ ] `/specs/phase-2/DEBT.md` exists with the 9-item seed block from
      architecture v2.1 §0.
- [ ] `scripts/spec-check.sh` continues to pass.

## Non-goals

- Implementing any Phase 2 functionality. This story is doc-only.
- Updating BASELINE.md (already done in `1ab1377` as part of the v2.0
  architecture commit).

---

## Implementation steps

This story documents work that landed alongside the PM session. No
further steps required.

---

## Verification

```bash
test -f specs/phase-2/requirements.md
test -f specs/phase-2/DEBT.md
bash scripts/spec-check.sh
```

---

## Files changed

- `specs/phase-2/requirements.md` (new)
- `specs/phase-2/DEBT.md` (new)
- `specs/phase-2/stories/story-2.1.md` (new — this file)
- `specs/phase-2/stories/story-2.2.md` … `story-2.27.md` (new — sibling stories from this PM session)

---

## Spec references

- `/specs/phase-2/architecture.md` v2.1 (`9b8d92a`) — input
- `/prompts/bmad-pm.md` — PM persona protocol

---

## Commit message

```
docs(phase-2): PM stories — 27 stories (2.1-2.27) + requirements.md + DEBT.md

BMAD PM persona output for Phase 2. Translates the v2.1 architecture
into 27 stories sized 0.5-3 hours each, total ~59 hours across 5 weeks.

Parallelisable seams documented in requirements.md §4. 5 channels
ship as one struct each (1-3 trait impl). All Phase 1 DEBT carry-overs
(#3 / #9 / #14 / #15 + story-1.15 wiring) fold into Phase 2 stories.
Phase 0 DEBT #16 (workspace TTL cron) pays down in story 2.17.

refs: /specs/phase-2/requirements.md
refs: /specs/phase-2/stories/story-2.1.md
```

---

## Notes for next story (2.2)

The schema layer kicks off in 2.2: V006 migration creates `projects`
and `tasks` tables and adds `sessions.task_id`. ProjectStore +
TaskStore land in the same story. After 2.2 lands, 2.3 + 2.7 + 2.16 +
2.17 + 2.22 can run in parallel.
