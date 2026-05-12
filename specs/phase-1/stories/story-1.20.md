# Story 1.20 — Phase 1 E2E acceptance run (runtime verification only)

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 1.1 - 1.19 (everything)
> **Phase**: 1
> **Type**: test
> **Reads first**: `/specs/phase-1/requirements.md` §5 (acceptance
> criteria — verbatim), `/specs/phase-1/architecture.md` §11.3
> (E2E spec), §11.4 (live-LLM smoke).

---

## Goal

Land the **runtime verification half** of Phase 1's closing acceptance:
the GAIA Level-1 fixture corpus, the deterministic 50-step synthetic
task that runs on the default `cargo test` path, and the
`workflow_dispatch` CI job that exercises both against real models when
keys are present. The **closeout/docs half** (retrospective, DEBT
audit, status flips, BASELINE/CHANGELOG) is story 1.20b. Splitting
keeps each piece in the 1-3h envelope and lets Codex review the test
infrastructure cleanly before the doc-level Phase-1-close commit.

## Acceptance criteria

- [ ] `crates/seasoned-hand-server/tests/phase1_gaia.rs` runs 10
      curated tasks loaded from `tests/fixtures/phase1_gaia/*.json`;
      each task is a `task_create` + a hand-picked correctness check
      (expected substring(s) in the final message). The test reports
      pass/fail per task and logs the count.
- [ ] **The fixture corpus exists in the repo** (deterministic). The
      `phase1_gaia` test itself is `#[ignore]`'d by default and gated
      behind `SEASONED_HAND_PHASE1_SMOKE=1`. When run in that mode it
      asserts the aggregate **total ≥ 8** (requirements.md §5 bar);
      otherwise it is skipped on local `cargo test`. Rationale:
      pass-rate against real cloud LLMs is environment-dependent;
      making it a hard CI gate would flake.
- [ ] `crates/seasoned-hand-server/tests/phase1_stable_50step.rs` runs
      on the **default** `cargo test` path (NOT `#[ignore]`) using a
      wiremock'd Bifrost that scripts a deterministic ≥50-step task.
      Asserts:
      - `state == FINISHED`.
      - No `Misc{kind:"stuck_terminate"}`,
        `Misc{kind:"max_steps_reached"}`, `Misc{kind:"cost_cap"}`.
      - Exactly one `Misc{kind:"verifier_verdict"}` with verdict =
        `pass` and `trigger_kind == "TaskComplete"`.
      - Wall-clock budget skipped on the default path (wiremock is
        instant); enforced only when `SEASONED_HAND_PHASE1_SMOKE=1`.
- [ ] `.github/workflows/ci.yml` gains a `workflow_dispatch` job
      `phase1-live-smoke` that:
      - Runs both `phase1_gaia` and `phase1_stable_50step` against
        real Bifrost when `ANTHROPIC_API_KEY` and `OPENAI_API_KEY`
        repo secrets are present.
      - Sets `SEASONED_HAND_PHASE1_SMOKE=1` so the aggregate
        threshold (≥8/10) and wall-clock budget are enforced.
      - Caps per-task spend at `cost_cap_cents=50` and the job
        timeout at 10 minutes.
      - **Not** in the default `ci` matrix.
- [ ] Tests pass on the default `cargo test --workspace` path without
      any cloud credentials.

## Non-goals

- Retrospective document, DEBT audit, requirements/BASELINE/CHANGELOG
  status updates — all story 1.20b.
- New features.
- Performance tuning beyond requirements.md §2 budgets.
- Multi-environment compatibility.
- Frontend automated tests (phase-1/DEBT.md #9 — Phase 2).

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

Each fixture is JSON:

```json
{
  "title": "GitHub stars of FoundationAgents/OpenManus",
  "briefing": "Find the GitHub star count of FoundationAgents/OpenManus and report it.",
  "expected_in_final_message": ["FoundationAgents/OpenManus"],
  "max_steps": 30,
  "cost_cap_cents": 30
}
```

`tests/common/gaia.rs` loads the JSON, drives `task_create` via the
existing Phase 0 WS test client, and reports per-task pass/fail.

### 2. Stable-50-step test

A purely-synthetic, network-free task: "Generate 12 numbered pages in
`/workspace/pages/p<N>.md` (each ~200 words), then read each,
summarize, concatenate to `/workspace/summaries.md`." Designed to
exceed 50 file_read/file_write cycles. Driven by a wiremock'd Bifrost
that scripts the tool-call sequence deterministically.

### 3. `workflow_dispatch` CI job

```yaml
# .github/workflows/ci.yml addition
phase1-live-smoke:
  if: github.event_name == 'workflow_dispatch'
  runs-on: ubuntu-latest
  timeout-minutes: 10
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - name: Live LLM smoke
      env:
        ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
        OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
        SEASONED_HAND_PHASE1_SMOKE: "1"
      run: |
        cargo test -p seasoned-hand-server --test phase1_gaia -- --ignored --nocapture
        cargo test -p seasoned-hand-server --test phase1_stable_50step -- --nocapture
```

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-server --test phase1_stable_50step   # default path
cargo test -p seasoned-hand-server --test phase1_gaia            # skipped without env
./scripts/spec-check.sh
```

Real-LLM smoke (manual):

```bash
gh workflow run phase1-live-smoke
```

---

## Files changed

- `crates/seasoned-hand-server/tests/phase1_gaia.rs` (new)
- `crates/seasoned-hand-server/tests/phase1_stable_50step.rs` (new)
- `crates/seasoned-hand-server/tests/common/gaia.rs` (new)
- `crates/seasoned-hand-server/tests/fixtures/phase1_gaia/*.json` (10 new)
- `.github/workflows/ci.yml` (modify — add `phase1-live-smoke`)

---

## Spec references

- `/specs/phase-1/requirements.md` §5 (acceptance — half satisfied
  here; the docs half lands in 1.20b).
- `/specs/phase-1/architecture.md` §11.3 (E2E), §11.4 (live-LLM smoke).
- `/specs/06-roadmap/ROADMAP.md` §"Phase 1" (acceptance bar).

---

## Commit message

```
test(phase-1): story 1.20 - E2E runtime verification (GAIA + 50-step + dispatch job)

- 10-task GAIA Level 1-style fixture at
  crates/seasoned-hand-server/tests/fixtures/phase1_gaia/; aggregate
  ≥8/10 threshold gated behind SEASONED_HAND_PHASE1_SMOKE=1
  (workflow_dispatch only — pass-rate against real LLMs is
  environment-dependent, not a default CI gate)
- phase1_stable_50step on the default cargo test path: wiremock'd
  Bifrost scripts a deterministic ≥50-step task; asserts FINISHED,
  no stuck/max-steps/cost terminations, exactly one TaskComplete
  verifier_verdict with pass, no wall-clock assertion on the wiremock
  path
- phase1-live-smoke workflow_dispatch CI job: runs both tests against
  real Bifrost when ANTHROPIC_API_KEY + OPENAI_API_KEY secrets are
  present; $0.50 per-task cap, 10-minute timeout; not in default ci

refs: /specs/phase-1/stories/story-1.20.md
```

---

## Notes for next story (1.20b)

Runtime verification infrastructure is in place. Story 1.20b writes
`specs/phase-1/RETROSPECTIVE.md`, runs the Phase 0 DEBT audit
(#18 / #21 / #22 / #25 / #27 strike-throughs), flips
`requirements.md` §4 statuses to `done`, updates `BASELINE.md`'s
Status line, and adds the versioned CHANGELOG entry. Pure-doc commit;
no code change.
