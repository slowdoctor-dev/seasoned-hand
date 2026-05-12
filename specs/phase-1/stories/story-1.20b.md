# Story 1.20b — Phase 1 closeout (retrospective + DEBT audit + status flips)

> **Status**: ready
> **Estimated**: 1.5 hours
> **Dependencies**: 1.20 (E2E runtime verification landed — supplies
> "Phase 1 by the numbers" data for the retrospective)
> **Phase**: 1
> **Type**: doc
> **Reads first**: `/specs/phase-0/RETROSPECTIVE.md` (template +
> tone), `/specs/phase-0/DEBT.md` (entries to close), `/specs/phase-1/DEBT.md`
> (seed + any in-phase additions), `/specs/phase-1/requirements.md` §5.

---

## Goal

Phase 1's pure-docs closeout: write the retrospective, close the
five Phase 0 DEBT items Phase 1 paid down (#18 / #21 / #22 / #25 /
#27), flip every Phase 1 story status to `done` in
`requirements.md` §4, update `BASELINE.md`'s Status line, and append
the versioned Phase 1 entry to `CHANGELOG.md`. No code; this is a
single-commit doc-only PR that closes the phase.

## Acceptance criteria

- [ ] `specs/phase-1/RETROSPECTIVE.md` written following the Phase 0
      template structure exactly: `What shipped` (commit table for
      stories 1.1 - 1.20 + 1.9b / 1.13b / 1.20b), `Deferred to Phase
      2+` (headline list from phase-1/DEBT.md), `What worked`
      (numbered list), `What to fix in Phase 2 (top 3)`, `Phase 1
      starting point — Phase 2`, `Phase 1 by the numbers`,
      `Corrections (post-close review)` (empty section to be filled
      after Codex post-close review per the Phase 0 pattern).
- [ ] `specs/phase-0/DEBT.md` items #18 / #21 / #22 / #25 / #27 each
      have a strike-through header (`### ~~#N. Title~~ ✅ resolved
      <YYYY-MM-DD> (story 1.X)`) with the implementation commit ref
      appended.
- [ ] `specs/phase-1/DEBT.md` audited: each entry from the
      architecture-phase seed (items 1-13) reviewed and either
      annotated `still-open` with a date, struck-through if resolved
      in-phase, or supplemented with a follow-up line. Any new debt
      introduced during stories 1.1 - 1.20 implementation is appended
      to the ledger (real entries, not the seed).
- [ ] `specs/phase-1/requirements.md` §4 table: **every** Status cell
      flipped to `done`. The table also reflects the 23-story final
      count (20 + 1.9b + 1.13b + 1.20b).
- [ ] `BASELINE.md` §1 "Status" row flipped from
      `Phase -1 (planning complete) → Phase 0 starting` to
      `Phase 1 complete → Phase 2 starting`.
- [ ] `CHANGELOG.md`: under a new `## [v0.1.0] - <YYYY-MM-DD>`
      heading (or the next semantic version), bullet the Phase 1
      headline changes (Verifier, Plan Manager, Initializer + Worker,
      4-condition Circuit Breaker, Checkpoint Manager, Narrator,
      3-track BrowserTab, WS task-control closure).
- [ ] No regressions: `cargo clippy --all-targets -- -D warnings`,
      `cargo fmt --check`, `cargo test --workspace`, and
      `./scripts/spec-check.sh` all green after the commit.

## Non-goals

- New tests, new fixtures, new CI jobs — all story 1.20.
- Architecture changes (Phase 1 is closing).
- ADR-011 / ARCHITECTURE.md v1.1 for the tool-count drift
  (phase-1/DEBT.md #1) — that's a separate doc-only commit not part
  of phase close.
- Phase 2 planning artifacts.

## Implementation steps

### 1. RETROSPECTIVE.md skeleton

Copy `specs/phase-0/RETROSPECTIVE.md`'s section headers; fill each
section per phase reality. The table of stories pulls one row per
story-1.X.md plus 1.9b / 1.13b / 1.20b. Commit refs come from
`git log --grep "story 1\." --oneline` filtered to the Phase 1
commit range.

`Phase 1 by the numbers` includes: total stories, total tests count
(`cargo test --workspace -- --list | wc -l`), DEBT items
opened-vs-closed, Verifier coverage observed during E2E (read from
the `phase1_stable_50step` test output).

`Corrections (post-close review)` left empty with a one-line
placeholder: `_To be filled when Codex completes the post-close
review (Phase 0 pattern: review at fbb562f surfaced 4 items → fixed
in 3426780)._`

### 2. DEBT close-outs

In `specs/phase-0/DEBT.md`:

```
### ~~18. SandboxClient holds in-process handle cache — single-process assumption~~ ✅ resolved 2026-XX-XX (story 1.2, commit <sha>)
### ~~21. Hook output-truncation path falls back to inline preview~~ ✅ resolved 2026-XX-XX (story 1.14, commit <sha>)
### ~~22. Capability table assumes Bifrost cloud aliases support tool calling~~ ✅ resolved 2026-XX-XX (story 1.7, commit <sha>)
### ~~25. Plan tools remain callable stubs~~ ✅ resolved 2026-XX-XX (story 1.1, commit <sha>)
### ~~27. WS task_pause/task_resume/task_cancel are protocol stubs~~ ✅ resolved 2026-XX-XX (story 1.17, commit <sha>)
```

Date format: ISO-8601. Commit SHAs come from `git log --oneline |
grep "story 1.<N>"`.

### 3. Requirements status flip

In `specs/phase-1/requirements.md` §4 table, add a `Status` column
(or update existing one) so every row reads `done`. Re-validate
totals row at the bottom (`23 stories, ~50 h` post-1.20b split).

### 4. BASELINE update

```diff
- | **Status** | Phase -1 (planning complete) → Phase 0 starting |
+ | **Status** | Phase 1 complete → Phase 2 starting |
```

### 5. CHANGELOG entry

Follow KeepAChangelog format. Group under `### Added`, `### Changed`,
`### Fixed`. Reference the architecture spec commit (`3f2fa6c`) and
the PM-stories commit (`3eb0c30`) for traceability.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
./scripts/spec-check.sh
grep -c "✅ resolved" specs/phase-0/DEBT.md   # expect ≥ 5 new strike-throughs
grep -c "| done " specs/phase-1/requirements.md  # expect 23
```

Manual: read RETROSPECTIVE.md end-to-end; ensure every section is
filled and the commit table is complete.

---

## Files changed

- `specs/phase-1/RETROSPECTIVE.md` (new)
- `specs/phase-0/DEBT.md` (modify — close #18 / #21 / #22 / #25 / #27)
- `specs/phase-1/DEBT.md` (modify — audit + append any in-phase debt)
- `specs/phase-1/requirements.md` (modify — flip every Status to `done`)
- `BASELINE.md` (modify — Status row)
- `CHANGELOG.md` (modify — versioned entry)

---

## Spec references

- `/specs/phase-0/RETROSPECTIVE.md` (template + tone — verbatim
  section headers).
- `/specs/phase-1/requirements.md` §5 (acceptance — verified by 1.20;
  documented here).
- `/specs/phase-1/DEBT.md` (audit target).

---

## Commit message

```
docs(phase-1): story 1.20b - Phase 1 close (retrospective + DEBT audit + status flips)

- specs/phase-1/RETROSPECTIVE.md written following Phase 0 template:
  What shipped (23-story commit table) / Deferred / What worked /
  What to fix in Phase 2 (top 3) / Phase 1 starting point - Phase 2 /
  Phase 1 by the numbers / Corrections (post-close review)
  placeholder
- specs/phase-0/DEBT.md: items #18 / #21 / #22 / #25 / #27 struck
  through with story refs and commit SHAs
- specs/phase-1/DEBT.md: 13-item seed audited; in-phase additions
  appended (real entries from stories 1.1-1.20, not the seed block)
- specs/phase-1/requirements.md §4: every Status cell flipped to
  done; total reflects 23-story final count
- BASELINE.md §1 Status row: "Phase 1 complete → Phase 2 starting"
- CHANGELOG.md: [v0.1.0] - <YYYY-MM-DD> entry with grouped Added /
  Changed / Fixed bullets

Phase 1 closed. Next BMAD step: fresh session with Architect persona
on /specs/phase-2/architecture.md per /specs/06-roadmap/ROADMAP.md
§Phase 2.

refs: /specs/phase-1/stories/story-1.20b.md
```

---

## Notes for next phase (Phase 2)

This commit is the natural Phase-1-close branch point. Branch off
`main` at this SHA. Start a fresh AI session with the BMAD Architect
persona on `/specs/phase-2/architecture.md` (per ROADMAP: briefing
protocol, Project/Task/Subtask model, pause/resume across days,
async notifications).

Top three Phase 1 carry-overs to revisit in Phase 2:

- phase-1/DEBT.md #3 (Verifier rollback opt-in default) — decide
  based on Phase 1 verifier precision data captured in RETROSPECTIVE.md.
- phase-1/DEBT.md #9 (frontend automated tests) — Phase 2 brings up
  Playwright / Vitest+RTL.
- phase-1/DEBT.md #4 (single invalidation heuristic) — add a second
  heuristic if Phase 1 retro surfaces false-negative tasks.
