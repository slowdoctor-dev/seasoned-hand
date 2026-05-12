# Story 1.20 — Phase 1 E2E + acceptance fixture + live-LLM smoke + retrospective

> **Status**: ready
> **Estimated**: 3-4 hours
> **Dependencies**: 1.1 - 1.19 (everything)
> **Phase**: 1
> **Type**: test + doc
> **Reads first**: `/specs/phase-1/requirements.md` §5 (acceptance
> criteria — verbatim), `/specs/phase-1/architecture.md` §11.3
> (E2E + closing story spec), §11.4 (live-LLM smoke),
> `/specs/phase-0/RETROSPECTIVE.md` (template for the closing retro).

---

## Goal

Close Phase 1 with a concrete acceptance run:

1. A **GAIA Level 1-style 10-task fixture** committed to the repo;
   `cargo test -p seasoned-hand-server --test phase1_gaia` runs them
   and reports pass/fail per task. **≥ 8 of 10 must pass.**
2. A **50-step synthetic stable task** (curated: multi-page browse +
   summarize) runs end-to-end without Stuck / MaxSteps / Cost
   termination and without going > $2.
3. A **live-LLM `workflow_dispatch` CI job** runs a single 50-step
   task against real Bifrost + real models when keys are present, with
   $0.50 cap and 10-minute timeout. Not on default `cargo test`.
4. `phase-1/RETROSPECTIVE.md` written following the Phase 0 template
   (`What shipped`, `Deferred`, `What worked`, `What to fix`, `Phase 1
   by the numbers`).
5. `phase-0/DEBT.md` audit confirms #18 / #21 / #22 / #25 / #27 are
   struck through. `phase-1/DEBT.md` grows entries for any new
   shortcuts introduced during this phase's stories.

After this story, the requirements doc's §5 acceptance block is
verifiably satisfied and the phase closes.

## Acceptance criteria

- [ ] `crates/seasoned-hand-server/tests/phase1_gaia.rs` runs 10
      curated tasks (loaded from
      `tests/fixtures/phase1_gaia/*.json`); each task is a
      `task_create` + a hand-picked correctness check (e.g. expected
      substring in the final message). The test reports pass/fail per
      task and logs the count.
- [ ] **The fixture corpus exists in the repo** (deterministic). The
      `phase1_gaia` test itself is `#[ignore]`'d by default and gated
      behind `SEASONED_HAND_PHASE1_SMOKE=1`. When run in that mode it
      asserts the aggregate **total ≥ 8** as the bar from
      requirements.md §5; otherwise it is skipped on local
      `cargo test`. Rationale: pass-rate against real cloud LLMs is
      environment-dependent; making it a hard CI gate would flake.
- [ ] The default `cargo test --workspace` path runs *only* the
      synthetic 50-step test (see below), which is deterministic
      against a wiremock'd Bifrost and asserts the same goal of
      "50+ tool call sessions stable" without needing live keys.
- [ ] `crates/seasoned-hand-server/tests/phase1_stable_50step.rs`
      runs on the **default** `cargo test` path (NOT `#[ignore]`) using
      a wiremock'd Bifrost that scripts a deterministic ≥50-step task
      (e.g. "Read 12 numbered pages from /workspace/pages/, summarize,
      concatenate to /workspace/summaries.md"). Asserts:
      - Session terminates with `state == FINISHED`.
      - No `Misc{kind:"stuck_terminate"}`,
        `Misc{kind:"max_steps_reached"}`, `Misc{kind:"cost_cap"}` in
        the event stream.
      - Exactly one `Misc{kind:"verifier_verdict"}` with verdict =
        `pass` and trigger = `TaskComplete`.
      - Wall-clock budget check is skipped on the default path
        (wiremock is instant). The wall-clock assertion is enforced
        only when `SEASONED_HAND_PHASE1_SMOKE=1` is set and the test
        runs against real Bifrost — same gating as `phase1_gaia`.
- [ ] `.github/workflows/ci.yml` gains a `workflow_dispatch` job
      `phase1-live-smoke` that runs both the GAIA fixture and the
      stable 50-step test against real models when
      `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` are present in the
      repo's GitHub secrets. Cost cap $0.50 per run (uses
      `cost_cap_cents = 50`), 10-minute job timeout. NOT in the
      default `ci` matrix.
- [ ] `specs/phase-1/RETROSPECTIVE.md` written following the Phase 0
      template's section headers and tone (see file map below for
      exact sections).
- [ ] DEBT audit:
      - `specs/phase-0/DEBT.md`: items #18, #21, #22, #25, #27 all
        struck through with date + commit ref.
      - `specs/phase-1/DEBT.md`: append any new debt introduced during
        Phase 1 implementation that wasn't seeded. Each entry follows
        the seed template.
- [ ] `specs/phase-1/requirements.md` § 4 story-table column "Status"
      flipped to `done` for every story (in the same commit).
- [ ] `BASELINE.md` § 1 "Status" field flipped from "Phase -1 (planning
      complete) → Phase 0 starting" to "Phase 1 complete → Phase 2
      starting".
- [ ] `CHANGELOG.md` gains a Phase 1 section under a new `[v0.1.0] -
      2026-MM-DD` heading (or whichever version your team is bumping
      to), summarizing the major changes (Verifier, Plan Manager,
      Initializer, Circuit Breaker, Checkpoints, Narrator, 3-track,
      task-control closure).

## Non-goals

- New features. This story is closure only.
- Adding > 10 fixture tasks. The "≥ 8 / 10" gate is the architecture-
  stipulated bar; more is Phase 4 retrospective work.
- Performance tuning beyond the budgets in requirements.md §2.
- Multi-environment compatibility testing (Phase 5).
- Frontend automated tests — explicitly deferred (phase-1/DEBT.md #9).

## Implementation steps

### 1. Fixture corpus

Create `crates/seasoned-hand-server/tests/fixtures/phase1_gaia/`:

```
01_github_stars.json
02_summarize_paper_first_page.json
03_find_definition_in_repo.json
04_extract_table_from_wikipedia.json
05_count_occurrences_in_text.json
06_unit_convert_via_browser.json
07_summarize_readme.json
08_locate_file_by_pattern.json
09_pull_release_date.json
10_aggregate_csv_in_workspace.json
```

Each fixture is a JSON document:

```json
{
  "title": "GitHub stars of FoundationAgents/OpenManus",
  "briefing": "Find the GitHub star count of FoundationAgents/OpenManus and report it.",
  "expected_in_final_message": ["FoundationAgents/OpenManus", "★"],
  "max_steps": 30,
  "cost_cap_cents": 30
}
```

The fixture loader is a `tests/common/gaia.rs` helper that parses the
JSON and runs `task_create` via the existing Phase 0 WS test client.

### 2. Stable-50-step test

A purely-synthetic task that does not require external network:
"Generate 12 numbered pages locally in /workspace/pages/p<N>.md (each
~200 words), then read each, summarize, and concatenate the summaries
into /workspace/summaries.md." Designed to exceed 50 file_read/file_write
cycles.

### 3. Workflow

```yaml
# .github/workflows/ci.yml addition
phase1-live-smoke:
  if: github.event_name == 'workflow_dispatch'
  runs-on: ubuntu-latest
  timeout-minutes: 10
  steps:
    - uses: actions/checkout@v4
    - name: Install Rust
      uses: dtolnay/rust-toolchain@stable
    - name: Live LLM smoke
      env:
        ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
        OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
        SEASONED_HAND_PHASE1_SMOKE: "1"
      run: |
        cargo test -p seasoned-hand-server --test phase1_gaia -- --nocapture
        cargo test -p seasoned-hand-server --test phase1_stable_50step -- --nocapture
```

### 4. RETROSPECTIVE.md

Following the Phase 0 template exactly:

```
# Phase 1 — Retrospective

> Phase 1 shipped <DATE>. 20 stories, ~hours of wall time, two agents
> (Claude Code + Codex CLI) in parallel tmux panes.

## What shipped

(table: 20 rows — Story 1.X / Title / Commit)

## Deferred to Phase 2+

(headline list by severity from phase-1/DEBT.md)

## What worked

1. ...
2. ...

## What to fix in Phase 2 (top 3)

1. ...

## Phase 1 starting point — Phase 2

- Branch off `main` at the story-1.20 commit
- Read `specs/06-roadmap/ROADMAP.md` §"Phase 2"
- BMAD Architect persona → `/specs/phase-2/architecture.md`

## Phase 1 by the numbers

- 20 stories shipped
- N unit/integration tests passing
- M new DEBT items
- Verifier coverage: <% of tasks>
- ...

## Corrections (post-close review)

(populated after Codex post-close review, mirroring Phase 0 process)
```

### 5. DEBT audit commit

```bash
# In specs/phase-0/DEBT.md, ensure these are struck through:
- ~~#18 SandboxClient handle cache~~ ✅ resolved <date> (story 1.2)
- ~~#21 Hook output truncation~~ ✅ resolved <date> (story 1.14)
- ~~#22 Capability table fallback~~ ✅ resolved <date> (story 1.7)
- ~~#25 Plan Manager stubs~~ ✅ resolved <date> (story 1.1)
- ~~#27 WS task control stubs~~ ✅ resolved <date> (story 1.17)
```

### 6. requirements.md / BASELINE.md / CHANGELOG.md edits

Single-commit changes (this story's commit):

- `specs/phase-1/requirements.md` §4 table: flip every "Status" cell
  to `done`.
- `BASELINE.md` §1 Status row updated.
- `CHANGELOG.md` Unreleased → versioned entry under
  `[v0.1.0] - <date>`.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-server --test phase1_gaia                # local (mock or real)
cargo test -p seasoned-hand-server --test phase1_stable_50step       # local
./scripts/spec-check.sh
```

Real-LLM smoke (manual, after the workflow_dispatch is wired):

```bash
gh workflow run phase1-live-smoke
```

Watch the run; assert green at `phase1_gaia` (≥ 8 pass) and
`phase1_stable_50step` (single pass).

---

## Files changed

- `crates/seasoned-hand-server/tests/phase1_gaia.rs` (new)
- `crates/seasoned-hand-server/tests/phase1_stable_50step.rs` (new)
- `crates/seasoned-hand-server/tests/common/gaia.rs` (new)
- `crates/seasoned-hand-server/tests/fixtures/phase1_gaia/*.json`
  (10 new)
- `.github/workflows/ci.yml` (modify — add `phase1-live-smoke` job)
- `specs/phase-1/RETROSPECTIVE.md` (new)
- `specs/phase-0/DEBT.md` (modify — close #18 / #21 / #22 / #25 / #27)
- `specs/phase-1/DEBT.md` (modify — append any new items discovered
  during Phase 1)
- `specs/phase-1/requirements.md` (modify — flip Status column to done)
- `BASELINE.md` (modify — Status row)
- `CHANGELOG.md` (modify — versioned entry)

---

## Spec references

- `/specs/phase-1/requirements.md` §5 (acceptance — must be satisfied).
- `/specs/phase-1/architecture.md` §11.3 (E2E closing story), §11.4
  (live-LLM smoke job).
- `/specs/06-roadmap/ROADMAP.md` §"Phase 1" acceptance ("GAIA Level 1-
  style tasks succeed ≥ 80 %").
- `/specs/phase-0/RETROSPECTIVE.md` (structure / tone template).

---

## Commit message

```
test(phase-1): story 1.20 - E2E + acceptance fixture + retrospective

- 10-task GAIA Level 1-style fixture (≥ 8/10 must pass) at
  crates/seasoned-hand-server/tests/fixtures/phase1_gaia/
- phase1_stable_50step test: synthetic ≥50-step task asserts FINISHED
  state, no stuck/max-steps/cost terminations, exactly one
  TaskComplete-trigger verifier_verdict with pass verdict, wall-clock
  under the §7 budget when main=Claude-class
- phase1-live-smoke workflow_dispatch CI job: runs both tests against
  real Bifrost + real Anthropic/OpenAI keys when present; $0.50 cap,
  10-min timeout; NOT on default cargo test path
- phase-1/RETROSPECTIVE.md drafted per Phase 0 template
- Phase 0 DEBT #18, #21, #22, #25, #27 struck through with commit refs
- specs/phase-1/requirements.md §4 statuses flipped to done; BASELINE.md
  Status updated; CHANGELOG.md [v0.1.0] entry

Phase 1 closed.

refs: /specs/phase-1/stories/story-1.20.md
```

---

## Notes for next phase (Phase 2)

Branch off `main` at this commit. Start a fresh AI session with the BMAD
Architect persona pointed at `/specs/phase-2/architecture.md` (to be
created). Phase 2 deliverables per ROADMAP: briefing protocol, deliverable
templates, Project/Task/Subtask data model, pause/resume across days,
async notifications.

Top-three carry-overs from Phase 1 DEBT to revisit in Phase 2:

- #3 (Verifier rollback opt-in default) — decide based on Phase 1
  verifier precision data captured in this retrospective.
- #9 (frontend automated tests) — Phase 2 brings up Playwright /
  Vitest+RTL to cover the three new UI surfaces.
- #4 (single invalidation heuristic) — Phase 2 may add a second
  heuristic if Phase 1 retrospective surfaces false-negative tasks.
