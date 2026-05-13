# Story 2.27 — Phase 2 closeout (retrospective + DEBT audit + status flips)

> **Status**: ready
> **Estimated**: 1.5 hours
> **Dependencies**: 2.26
> **Phase**: 2
> **Type**: doc
> **Reads first**: `/specs/phase-1/RETROSPECTIVE.md` (template + tone),
> `/specs/phase-2/DEBT.md` (audit target), `/specs/phase-2/requirements.md` §5

---

## Goal

Pure-docs closeout for Phase 2. Same shape as Phase 1's story 1.20b.
One commit, no code change.

## Acceptance criteria

- [ ] `specs/phase-2/RETROSPECTIVE.md` written following the Phase 1
      template: "What shipped" (commit table for 27 stories),
      "Deferred to Phase 3+", "What worked", "What to fix in Phase 3
      (top 3)", "Phase 2 starting point — Phase 3", "Phase 2 by the
      numbers", "Corrections (post-close review)" placeholder.
- [ ] **Phase 1 DEBT items closed in Phase 2** stamped with their
      Phase-2 commit SHAs:
      - `#9` (FE automated tests) → story 2.24 commit
      - `#14` (SandboxGitShell shell-injection) → story 2.19 commit
      - `#15` (Worker XREADGROUP) → story 2.18 commit
- [ ] **Phase 0 DEBT #16** (workspace TTL cron) → story 2.17 commit.
- [ ] **Phase 1 DEBT #3 decision** (verifier rollback default flip):
      collect verdict precision from `phase2-live-overnight` runs
      (story 2.26) — if precision ≥90%, flip default to `true` in
      this commit; else add "carried to Phase 3" note to Phase 2
      DEBT.md. Document the decision verbatim in the retrospective.
- [ ] `specs/phase-2/DEBT.md` audited:
      - Each seed item (1-9) annotated `still-open` with date OR
        strike-through if resolved in-phase
      - Any new debt introduced during 2.1-2.26 appended (real
        entries from per-story exec notes if surfaced)
- [ ] `specs/phase-2/requirements.md` §4 table: every Status flipped
      to `done`. Totals row updated to match 27.
- [ ] `BASELINE.md` §1 Status row: "Phase 2 complete → Phase 3 starting".
- [ ] `BASELINE.md` §6 Roadmap table: Phase 2 timeline column shows
      5 weeks (was 3 in ROADMAP) with a note.
- [ ] `CHANGELOG.md`: new `[v0.2.0] - <YYYY-MM-DD>` section under
      `[Unreleased]`. Phase 2 headline changes grouped Added /
      Changed / Fixed / Deferred (mirror v0.1.0 entry pattern).
- [ ] Verification: `cargo clippy --all-targets -- -D warnings`,
      `cargo fmt --check`, `cargo test --workspace`, `scripts/spec-check.sh`
      all green.

## Non-goals

- New tests / fixtures / CI jobs (story 2.26 was the cap).
- Architecture changes (Phase 2 is closing).
- ADR-011 for tool-count drift — separate doc-only commit; not part
  of phase close.
- Phase 3 planning artifacts (those are a Phase 3 BMAD Analyst
  output).

---

## Implementation steps

### 1. RETROSPECTIVE.md skeleton

Copy Phase 1 retrospective headers. Pull commit refs from
`git log --grep "story 2\." --oneline` filtered to the Phase 2
commit range.

`Phase 2 by the numbers`:
- 27 stories shipped
- Total tests (`cargo test --workspace -- --list | wc -l`)
- 5 weeks duration
- DEBT items opened vs closed
- Channels registered (5)
- Renderer formats supported (8)

### 2. DEBT close-outs

Phase 1 DEBT.md strike-throughs:
```
### ~~9. Frontend has no automated test coverage~~ ✅ resolved 2026-MM-DD (story 2.24, commit `XXXX`)
### ~~14. SandboxGitShell::commit_phase shell-quoting~~ ✅ resolved 2026-MM-DD (story 2.19, commit `XXXX`)
### ~~15. Verifier Worker run() is a polling stub~~ ✅ resolved 2026-MM-DD (story 2.18, commit `XXXX`)
```

Phase 0 DEBT.md strike-through:
```
### ~~16. Sandbox workspace cleanup is manual~~ ✅ resolved 2026-MM-DD (story 2.17, commit `XXXX`)
```

### 3. Phase 1 DEBT #3 decision

Per the retrospective:
- Pull verifier precision from `phase2-live-overnight` runs (count
  TaskComplete verdicts → manually mark pass/fail correctness)
- Document the number in `Phase 2 by the numbers` section
- If ≥90%, flip default in `seasoned-hand-server/src/lib.rs`:
  `checkpoint_rollback_on_verifier_fail = true`; strike-through Phase
  1 DEBT #3
- Else, append to Phase 2 DEBT.md as item 10 "Rollback default
  carried over from Phase 1 DEBT #3 — Phase 3 retro to revisit"

### 4. Requirements + BASELINE + CHANGELOG

Same pattern as Phase 1 1.20b commit `e5948c2`.

### 5. Verification

Run all gates; commit; push.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
./scripts/spec-check.sh
grep -c "✅ resolved" specs/phase-1/DEBT.md   # expect ≥ 3 new strike-throughs (#9 / #14 / #15)
grep -c "✅ resolved" specs/phase-0/DEBT.md   # expect 1 new strike-through (#16)
grep -c "| done " specs/phase-2/requirements.md  # expect 27
```

Manual: read RETROSPECTIVE.md end-to-end; ensure every section is
filled and the commit table is complete.

---

## Files changed

- `specs/phase-2/RETROSPECTIVE.md` (new)
- `specs/phase-0/DEBT.md` (modify — close #16)
- `specs/phase-1/DEBT.md` (modify — close #9 / #14 / #15; potentially
  #3 depending on precision data)
- `specs/phase-2/DEBT.md` (modify — audit + append any new in-phase
  debt)
- `specs/phase-2/requirements.md` (modify — every Status → done;
  totals refresh)
- `BASELINE.md` (modify — Status row + Roadmap line)
- `CHANGELOG.md` (modify — [v0.2.0] section)
- Possibly `crates/seasoned-hand-server/src/lib.rs` (modify — flip
  rollback default IF precision data justifies)

---

## Spec references

- `/specs/phase-1/RETROSPECTIVE.md` (template + tone — verbatim
  section headers)
- `/specs/phase-2/requirements.md` §5 (acceptance — verified by 2.25
  and 2.26; documented here)
- `/specs/phase-2/DEBT.md` (audit target)

---

## Commit message

```
docs(phase-2): story 2.27 - Phase 2 close (retrospective + DEBT audit + status flips)

- specs/phase-2/RETROSPECTIVE.md written following Phase 1 template:
  What shipped (27-story commit table) / Deferred / What worked /
  What to fix in Phase 3 (top 3) / Phase 2 starting point - Phase 3 /
  Phase 2 by the numbers / Corrections placeholder
- specs/phase-1/DEBT.md: items #9 / #14 / #15 struck through with
  Phase 2 story refs + commit SHAs. #3 (rollback default) handled
  per precision data (flip or carry).
- specs/phase-0/DEBT.md: item #16 struck through (story 2.17).
- specs/phase-2/DEBT.md: 9-item seed audited; in-phase additions
  appended (real entries from per-story exec notes, not the seed).
- specs/phase-2/requirements.md §4: every Status cell flipped to done;
  total reflects 27 stories shipped.
- BASELINE.md §1 Status: "Phase 2 complete → Phase 3 starting";
  §6 roadmap reflects 5-week timeline.
- CHANGELOG.md: [v0.2.0] - <YYYY-MM-DD> entry with grouped Added /
  Changed / Fixed bullets.

Phase 2 closed. Next BMAD step: fresh session with Architect persona
on /specs/phase-3/architecture.md (learning system, Curator) per
/specs/06-roadmap/ROADMAP.md §Phase 3.

refs: /specs/phase-2/stories/story-2.27.md
```

---

## Notes for next phase (Phase 3)

This commit is the natural Phase-2-close branch point. Branch off
`main` at this SHA. Start a fresh AI session with the BMAD Architect
persona on `/specs/phase-3/architecture.md` (per ROADMAP: 4-layer
learning system, playbook auto-extraction, skill library).

Top three Phase 2 carry-overs to revisit in Phase 3:

- phase-2/DEBT.md #2 (pre-baked sandbox image) — Phase 3 may bundle
  with whatever Phase 3 adds to the sandbox-side toolchain
- phase-2/DEBT.md #6 (empty skill/playbook tables) — Phase 3 fills
  these; it's the headline Phase 3 outcome
- phase-2/DEBT.md #7 (verifier rollback default decision) — only
  if 2.27 carried it instead of flipping
