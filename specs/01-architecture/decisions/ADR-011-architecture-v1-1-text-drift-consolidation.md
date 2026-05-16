# ADR-011: ARCHITECTURE.md v1.0 → v1.1 — text-drift consolidation

Status: Accepted
Date: 2026-05-16
Deciders: Project lead

## Context

`/specs/01-architecture/ARCHITECTURE.md` was frozen at v1.0 during the
Phase -1 planning kickoff. Three phases of execution have since shipped
(Phase 0 / 1 / 2 — closed 2026-05-12 / 2026-05-13 / 2026-05-15) and
the immutable doc text never updated. The drifts accumulated as
documented in Phase 1 DEBT #1, #2 and the pre-Phase-3 cross-phase
review at `/specs/REVIEW.md` (DEBT #51):

1. **§2.2 sessions states** lists 5 (`IDLE, RUNNING, FINISHED, ERROR,
   SUSPENDED`). Phase 1 story 1.9 widened the `sessions.state` CHECK
   constraint to 6 via `migrations/V004__verifications.sql:25-47`,
   adding `VERIFYING`. Phase 1 code writes it at
   `crates/seasoned-hand-core/src/agent/mod.rs:614`.
2. **`tasks` table is not in the immutable doc at all**. Phase 2
   introduced `tasks` (`migrations/V006__phase2_projects_tasks.sql`) as
   the durable user-facing unit on top of `sessions`. The 8-variant
   `TaskStatus` state machine (`Drafted → Briefed → Confirmed →
   Running ⇄ Paused → Completed | Failed | Cancelled`) is
   `crates/seasoned-hand-core/src/project/task.rs::legal_transitions`.
   v1.0 talks only about `sessions.state`.
3. **§2.4 + §7 tool count** says 32 (29 Manus + 3 learning). The
   shipped count is 38: Phase 1 added `feature_mark_done`,
   `progress_update`, `checkpoint_label`, `checkpoint_rollback`; Phase 2
   added `task_deliver`. `scripts/spec-check.sh` already pins the new
   count; the doc text never caught up.
4. **BASELINE.md §4 frontend stack** says "Next.js 15 + Tailwind v4 +
   React 19". Phase 0 story 0.18 shipped Next.js 16 / React 19.2 /
   Tailwind 4.3 (Phase 0 DEBT #27, retained as Phase 1 DEBT #2).

The §9 NEVER rule in `AGENTS.md` blocked unilateral edits. The
pre-Phase-3 review (`324e946`) surfaced the four drifts as the
consolidated **DEBT #51**; this ADR is the human-approved go to
reconcile them in one PR.

## Decision

Bump `/specs/01-architecture/ARCHITECTURE.md` from v1.0 to **v1.1**.
The v1.1 amendments are surgical — they reconcile text with shipped
reality; no new architectural choice. Specifically:

1. **§2.2 Sessions**: add `VERIFYING` to the state list (already in the
   schema since V004). Append a new **§2.2.1 Tasks** subsection
   describing the Phase 2 `tasks` table + 8-variant `TaskStatus` state
   machine, with a cross-ref to `/specs/phase-2/architecture.md` §2.2
   for the full transition matrix.
2. **§2.4 Tool Catalog**: bump "32 tools = 29 + 3" to
   "38 tools = 29 + 3 + 5 phase-additions" with a one-line breakdown of
   the Phase 1 + Phase 2 net adds.
3. **§7 Tool catalog (32 tools)**: rename heading to "Tool catalog (38
   tools)"; add a small subsection enumerating the Phase 1 + Phase 2
   additions with their story references.
4. **`/BASELINE.md` §4**: flip "Next.js 15" → "Next.js 16" in the
   stack table.

No code change is required for any of these — code already reflects
v1.1 reality. This is purely a documentation reconciliation.

## Consequences

**Positive:**
- The immutable doc once again matches the running system. Fresh AI
  sessions reading `ARCHITECTURE.md` no longer get misinformation
  about sessions vs tasks, tool count, or stack versions.
- Phase 1 DEBT #1, #2 + Phase 2 REVIEW DEBT #51 all close in one PR.
- Phase 3 starts on a clean text/code boundary.

**Negative:**
- v1.1 introduces a precedent for "drift consolidation ADRs" — every
  3-4 phases an ADR like this is likely to recur. Acceptable cost so
  long as each one is small and tied to a concrete drift list.

**Neutral:**
- Future drifts now have a clear pay-down shape (open DEBT entry,
  surface in next phase REVIEW, consolidate in a single ADR + version
  bump). This ADR is the template for that flow.

## Alternatives considered

### Alternative A: Skip the version bump; just amend ARCHITECTURE.md
The §9 NEVER rule was written specifically to prevent silent edits to
the immutable doc. Skipping the version bump would replay the same
mistake.

### Alternative B: Bump to v2.0 with a wider rewrite
v2.0 would imply a substantive architectural shift. None of the
v1.1 changes shift architecture — they reconcile text with what
already exists. v1.1 (minor) is the honest version label.

### Alternative C: Defer to Phase 3 architecture pass
Phase 3 starts with BMAD Architect on `/specs/phase-3/architecture.md`;
that pass would naturally re-read the v1.0 doc and surface the
drifts. Choosing to consolidate now (rather than defer) keeps the
fresh-context-per-story property: a Phase 3 story should not have to
correct the immutable doc as part of doing its narrower work.

## References

- `/specs/REVIEW.md` §3 Section A — drift catalogue (cross-phase
  pre-Phase-3 review, 2026-05-16)
- `/specs/phase-1/DEBT.md` #1, #2 — original drift entries
- `/specs/phase-2/DEBT.md` #51 — consolidated ledger entry
- `migrations/V004__verifications.sql:25-47` — VERIFYING state widening
- `migrations/V006__phase2_projects_tasks.sql` — tasks table
- `crates/seasoned-hand-core/src/project/task.rs::legal_transitions` —
  TaskStatus state machine
- `scripts/spec-check.sh:65-72` — tool-count pin
- ADR-010 (Plan as PCB) — preceding ADR
- `/AGENTS.md` §9 NEVER — the constraint that gates this ADR
