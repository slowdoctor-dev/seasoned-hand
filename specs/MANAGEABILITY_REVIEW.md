# Manageability + Reusability Review Log

Cross-phase audit trail for codebase manageability and reusability,
with tech-debt reduction as the throughline. Mirrors the format of
`SECURITY_REVIEW.md` — findings recorded chronologically with
severity, fix commit, and saturation notes.

This log lives outside `/specs/phase-N/` because the findings touch
Phase 2 scaffolding, Phase 3 spec inputs, and Phase 4 utility code
simultaneously.

---

## Audit cycle — 2026-05-20 (Claude solo)

> Reviewer: Claude solo (Codex on 5-day rate-limit recovery)
> Scope: post-Phase-4-close-out tech-debt reduction sweep
> Method: grep-based DRY-violation map → targeted code reads → fix
> commits → saturation re-sweep
> Net result: 5 iterations, -303 LOC of dead/duplicated code,
> 1 new shared utility module, ~10 spec-doc cross-ref repairs

### Surfaces probed

| Surface | Verdict |
|---|---|
| `TODO` / `FIXME` / `HACK` markers in production | 0 found |
| `unimplemented!` / `todo!` / `unreachable!` in production | 0 found |
| `.unwrap()` in non-`#[cfg(test)]` production code | 0 found (all hits are inside test blocks) |
| Largest source files (top is curator/mod.rs at 6011 lines) | accepted — each component is internally clean; splitting would conflict with active branches |
| Unused cargo dependencies (heuristic via `use` grep) | 0 found |
| Cargo build warnings | 0 |
| Cross-doc references to deleted symbols | 4 found, 4 fixed (iter-5) |

### F1 (M) — `skill::{SkillStore, PlaybookStore}` dead scaffolding

**Status**: FIXED at commit `e004b2d` (manageability iter-1)

**Threat model**: Phase 2 story 2.3 introduced these as reservation
handles for Phase 3 Curator wiring. Phase 3 (playbook extraction) and
Phase 4 (curator) both ended up writing through `crate::playbooks::*`
and direct `DbPool` access — the reservation slots were never used.
The empty wrappers carried `#[allow(dead_code)]` markers hiding the
fact that `pool: DbPool` was write-only.

**Fix**: deleted the `crate::skill` module entirely (mod.rs + tests.rs).
`AppState::{skills, playbooks}` fields and their boot-time constructor
calls dropped. The V009 schema tables (`skills`, `playbooks`) remain
in use — only the empty Rust wrappers went away.

Co-removed: 4 unused helpers in `cli::commands::init` (`workspace_root`,
`deliverables_dir`, `config_dir`, `exists`) and the `Candidate::status_raw`
+ `Candidate::updated_at` write-only fields in `task::ttl`. Total
`#[allow(dead_code)]` markers in production: 8 → 0.

**Net diff**: 7 files, -125 lines.

### F2 (L) — `now_micros` duplicated across 22 files

**Status**: FIXED at commit `578ad9b` (manageability iter-2)

**Threat model**: every persisted timestamp in the project is `i64`
microseconds since epoch (SQLite `INTEGER`). 22 files each carried
their own `fn now_micros() -> i64`. The bodies had drifted across
three subtle variants — saturating `i64::try_from(...).unwrap_or(i64::MAX)`,
wrapping `d.as_micros() as i64`, and the CLI's manual
`(secs * 1_000_000 + subsec_micros)`. All return identical values until
the year-294247 overflow horizon, but the drift would have grown if
left.

**Fix**: new `crate::time::{now_micros, now_micros_str}` with the
saturating shape. 22 local impls deleted; call sites import the
canonical helper. `agent::init::progress::now_micros_str` is now a
local thin wrapper for its own internal use; the temporary `pub use`
re-export it carried in iter-2 was removed in iter-4.

Four modules keep their own `fn now_micros() -> Result<i64, ErrorType>`
because they distinguish clock-error reporting through their
module-specific Error enum — that's a different contract (`Result<i64, _>`
vs `i64`) and not a DRY violation: `curator::CuratorWorkerError`,
`events::EventError`, `plan::PlanError`, plus one rollback-context
helper in `tools::builtin`.

**Net diff**: 25 files, +59 / -202 lines.

### F3 (L) — Phase 4 DEBT.md entry status flags out of sync with close-out matrix

**Status**: FIXED at commit `28f49d5` (manageability iter-3)

**Threat model**: the F-4.26 close-out matrix in `specs/phase-4/DEBT.md`
declared 10 entries' final state, but each individual entry's
`**Status**: open` flag was never updated. A grep for `Status: open`
returned a misleading view of outstanding work for anyone not reading
the matrix at the bottom.

**Fix**: synced each Status line to the matrix verdict (closed /
partial / deferred to Phase 5) with a pointer back at the F-4.26 matrix
so future audits don't have to cross-reference two sections.

### F4 (L) — `agent::init::progress` carried a `pub use` shim from iter-2

**Status**: FIXED at commit `c886df5` (manageability iter-4)

**Threat model**: iter-2's consolidation left `agent::init::progress`
with a `pub use crate::time::{now_micros, now_micros_str};` line so
its one external caller in `tools::builtin` wouldn't need to update.
Pure indirection.

**Fix**: `tools::builtin` imports `crate::time::now_micros` directly;
the re-export becomes a local `use crate::time::now_micros_str;` for
progress's own internal use.

### F5 (L) — Phase 2/3 specs referenced deleted `crate::skill` symbols

**Status**: FIXED at commit `0117acc` (manageability iter-5)

**Threat model**: iter-1 deleted `crate::skill`, but four spec files
still named it as if alive:
- `specs/phase-2/architecture.md` §2.12 + §3 (AppState field list)
- `specs/phase-2/stories/story-2.3.md` (acceptance criteria)
- `specs/phase-3/INPUTS.md` (Phase 3 inputs table)
- `specs/phase-3/stories/story-3.7.md` (test command in Verification
  block — `skill::outcome_counter_updates` no longer existed)

Per AGENTS.md §8 / NEVER list: "Never let code and spec drift silently."

**Fix**: each location now carries a Phase 4 manageability hardening
closure note. Original acceptance criteria left intact as the historical
record of what landed at the original commit; new sections explain the
later removal. Story 3.7's Verification block now points at the
relocated tests (`verifier::gate::tests::*`) — both verified to pass.

### Iter-6 — saturation sweep

Probed:
- Cargo build warnings (0)
- `#[allow(dead_code)]` / `#[allow(unused)]` markers (0 in production)
- Unused cargo deps via `use`-grep heuristic (0)
- Stale cross-doc refs (0 remaining)
- `unwrap` / `todo!` / `unimplemented!` / `unreachable!` outside tests (0)
- Public-API items potentially over-exposed (all justified)

**Findings**: zero new code fixes.

### Saturation verdict

Five iterations of concrete fixes, sixth iteration found zero
load-bearing items. Manageability + reusability hardening loop
saturates here.

Net tech-debt reduction across the cycle:
- 22 duplicated `now_micros` impls → 1 canonical helper
- 1 dead module (`crate::skill`) + 4 dead CLI helpers + 2 dead TTL
  fields gone (-125 LOC)
- 25 files touched by the consolidation, +59 / -202 LOC
- 10 DEBT.md status flags resynced with their close-out verdicts
- 4 spec files reconciled with the Phase 4 module-deletion

Codex review of this audit trail can land once the 5-day rate-limit
recovers; the loop is closed from Claude's side.
