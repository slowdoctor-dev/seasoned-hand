# Story CI — Validate + extend `.github/workflows/ci.yml`

> **Status**: done
> **Estimated**: 1 hour
> **Dependencies**: 0.27 (RETROSPECTIVE), 0.9b (full tool wiring)
> **Phase**: 0 (closeout — closes DEBT #14)
> **Type**: ops
> **Reads first**: existing `.github/workflows/ci.yml`, `specs/phase-0/DEBT.md` #14

---

## Goal

Validate the existing `.github/workflows/ci.yml` (carried over from
the initial scaffold) against the now-real workspace, fix anything
that doesn't run from a cold runner, and add coverage for the
`#[ignore]`-gated paths (Redis pub/sub, sandbox lifecycle) via opt-in
manual triggers. After this lands, every push to `main` actually
verifies what we claim is green.

**Why it's a closeout item**: Phase 0's retrospective overstated
gate-greenness because local cache hid a `cargo clippy --all-targets`
failure. CI from cold cache is the durable fix.

## Acceptance criteria

- [ ] `.github/workflows/ci.yml` runs end-to-end on `push: [main]`
      and `pull_request: [main]` against a fresh GitHub Actions
      `ubuntu-latest` runner with **no cached state preconditions**
- [ ] **Spec-check job** runs `./scripts/spec-check.sh` (already
      present in the existing workflow — verify still passes)
- [ ] **Rust job** runs, from cold:
      - `cargo fmt --all -- --check`
      - `cargo clippy --all-targets --workspace -- -D warnings`
      - `cargo test --workspace`
      - Remove the existing `--all-features` flag if no features are
        defined (don't lie to the runner)
- [ ] **Frontend job** runs `pnpm typecheck && pnpm lint && pnpm build`
      (current workflow runs `pnpm test` which is a no-op stub — keep
      it but add `pnpm build` for real coverage)
- [ ] **Optional ignored-tests job** (`workflow_dispatch` only — manual
      trigger, not on every push) that:
      - Brings up Redis via GitHub service container
      - Brings up Bifrost via service container (skipped if neither
        `ANTHROPIC_API_KEY` nor `OPENAI_API_KEY` secret is set)
      - Runs `cargo test -- --ignored` for the pubsub + sandbox-lifecycle
        + (eventually) `e2e_phase0` tests
- [ ] Workflow file checked in via PR; first green CI run on `main`
      captured in the PR description as evidence (or in DEBT.md closure)
- [ ] DEBT #14 closed with strike-through + date
- [ ] RETROSPECTIVE.md "Corrections" section gets a new bullet noting
      that CI now enforces the "all gates green" claim from this
      commit forward

## Non-goals

- Frontend E2E via Playwright (Phase 1)
- Multi-platform matrix (Phase 1 — Linux-only Phase 0)
- Custom self-hosted runners (never — GitHub-hosted is sufficient)
- Auto-merge / auto-deploy (Phase 6 release prep)
- Slow nightly runs (Phase 1 if needed)

---

## Implementation steps

### 1. Audit current workflow

`cat .github/workflows/ci.yml` — identify:
- Does `cargo test --workspace --all-features` need that flag? (Check
  if any `[features]` are declared in `Cargo.toml` files. If not, drop
  it — passing `--all-features` to a feature-less workspace is harmless
  but misleading.)
- Is `pnpm install --frozen-lockfile` the right thing? (Yes if lockfile
  is committed.)
- Is the markdownlint job worth keeping? (`continue-on-error: true`
  means it never blocks — keep it as informational.)

### 2. Push the existing workflow to trigger first real run

The current workflow has never run against the actual code. Commit
any audit fixes from step 1, then push and observe the first green
(or failing) run on GitHub Actions UI.

If it fails on cold cache: those failures are the real story — fix
them in this commit OR open per-failure follow-ups, depending on
severity.

### 3. Add ignored-tests job

```yaml
ignored-tests:
  name: Ignored tests (live Redis + sandbox)
  if: github.event_name == 'workflow_dispatch'
  runs-on: ubuntu-latest
  services:
    redis:
      image: redis:7-alpine
      ports: ['6379:6379']
    # Bifrost can be added similarly when we have a secret-gated path
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
    - name: Pull AIO Sandbox image (~1GB; cached across runs)
      run: docker pull ghcr.io/agent-infra/sandbox:1.0.0.152
    - name: Run ignored tests
      run: cargo test --workspace -- --ignored
      env:
        REDIS_TEST_URL: redis://localhost:6379
```

### 4. Close DEBT #14 + amend RETROSPECTIVE

Same pattern as the other closeout items.

---

## Files changed

- `.github/workflows/ci.yml` (modify — audit + add ignored-tests job)
- `specs/phase-0/DEBT.md` (close #14)
- `specs/phase-0/RETROSPECTIVE.md` (Corrections bullet)

---

## Spec references

- `specs/phase-0/DEBT.md` #14 (the entry being closed)
- `specs/phase-0/RETROSPECTIVE.md` "Corrections" section (where the new
  bullet lands)
- `AGENTS.md` §6 (the canonical "verification gates" list)

---

## Commit message

```
ops(phase-0): CI workflow validated + ignored-tests job

- Audit + fix .github/workflows/ci.yml against the now-real Rust
  workspace + Next.js frontend
- Frontend job: add pnpm build (real coverage, not just typecheck/lint)
- Drop misleading --all-features from cargo test (no features defined)
- New workflow_dispatch-only job: ignored-tests with Redis service
  container, runs cargo test -- --ignored
- First green CI run on main proves the gates from cold cache
- Closes DEBT #14; RETROSPECTIVE Corrections bullet added

refs: /specs/phase-0/stories/story-ci.md
```

---

## Definition of "Phase 0 truly closed"

After this story lands AND its first green CI run on `main`, the
phase is *honestly* done:

- ✅ 27 stories shipped (`fbb562f`)
- ✅ Post-review MAJORs fixed (`967445f`)
- ✅ 0.9b sandbox tools wired (`3b55cbe`)
- ✅ CI workflow live + green from cold cache (this story)

Then Phase 1 architecture work can begin in a fresh BMAD Architect
session, with the closeout commits and DEBT.md state as input.
