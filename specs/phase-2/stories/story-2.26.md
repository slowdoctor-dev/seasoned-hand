# Story 2.26 — Phase 2 live-LLM workflow_dispatch jobs

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 2.25
> **Phase**: 2
> **Type**: test
> **Reads first**: `/specs/phase-2/architecture.md` §11 (E2E), Phase 1 1.20 (template)

---

## Goal

Mirror Phase 1's `phase1-live-smoke` workflow_dispatch CI job for
Phase 2. Two new jobs run against real Bifrost + secrets when
manually triggered:

1. **`phase2-live-overnight`** — full "Do this overnight" flow on a
   real GAIA-Level-1-style task.
2. **`phase2-live-webhook-roundtrip`** — webhook intake → email
   delivery, exercising two distinct channels end-to-end.

## Acceptance criteria

- [ ] `crates/seasoned-hand-server/tests/phase2_live_overnight.rs` —
      `#[ignore]`'d by default; gated behind
      `SEASONED_HAND_PHASE2_SMOKE=1`. Same shape as Phase 1's
      `phase1_gaia.rs` but uses the briefing flow + durable pause +
      .docx deliverable + email reply.
- [ ] `crates/seasoned-hand-server/tests/phase2_webhook_roundtrip.rs` —
      `#[ignore]`'d; tests webhook intake → real briefing →
      task runs → email delivery to a configured address. Asserts the
      webhook callback URL received a POST with `{task_id,
      deliverable_id, status, content_url}`.
- [ ] `.github/workflows/ci.yml` gains two new
      `workflow_dispatch`-only jobs:
      - `phase2-live-overnight`: runs `phase2_live_overnight` with
        `SEASONED_HAND_PHASE2_SMOKE=1`, 30-min timeout, $1.50 per-task
        cap, requires `ANTHROPIC_API_KEY` + `OPENAI_API_KEY` secrets.
      - `phase2-live-webhook-roundtrip`: same env, requires also
        `SMTP_HOST` / `SMTP_USERNAME` / `SMTP_PASSWORD` /
        `WEBHOOK_CALLBACK_URL` secrets.
- [ ] Neither live job runs in the default CI matrix.
- [ ] Phase 1's `phase1-live-smoke` job stays as-is (unchanged).
- [ ] Each live test prints a final summary line `phase2 smoke pass:
      task_id=... deliverable_format=... wall_seconds=...` to stdout
      so the CI run page shows it without scrolling logs.

## Non-goals

- Cost / latency dashboarding (Phase 5).
- Multi-LLM-provider matrix testing (Phase 5).

---

## Implementation steps

### 1. phase2_live_overnight.rs

Adapt `phase2_overnight.rs` (story 2.25) — swap wiremocked Bifrost
for the real `BIFROST_BASE_URL`. Drop the `tokio::time::pause`
scaffolding (real clock; auto-confirm takes 5 min real). Cap
`max_steps = 80`, `cost_cap_cents = 150`.

Use a fixed test brief: "Generate a 3-page markdown summary of the
following git log. Run `git -C /workspace log --oneline | head -50`
and produce summary.docx with paragraphs grouping commits by week."

Assert: `task.status == completed`, exactly one TaskComplete pass
verdict, deliverable `.docx` exists, manifest valid.

### 2. phase2_webhook_roundtrip.rs

POST to `http://127.0.0.1:3000/v1/intake/webhook` with a brief and
`reply_target.channel = "email"`. Configure a test mailbox via env.
After task completion, verify a real email arrived in the test
mailbox (poll via async-imap on a real account).

### 3. CI workflow

```yaml
phase2-live-overnight:
  if: github.event_name == 'workflow_dispatch'
  runs-on: ubuntu-latest
  timeout-minutes: 30
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - name: Phase 2 overnight smoke
      env:
        ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
        OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
        SEASONED_HAND_PHASE2_SMOKE: "1"
      run: |
        cargo test -p seasoned-hand-server --test phase2_live_overnight -- --ignored --nocapture

phase2-live-webhook-roundtrip:
  if: github.event_name == 'workflow_dispatch'
  runs-on: ubuntu-latest
  timeout-minutes: 15
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - name: Phase 2 webhook + email roundtrip
      env:
        ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
        OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
        SMTP_HOST: ${{ secrets.SMTP_HOST }}
        SMTP_USERNAME: ${{ secrets.SMTP_USERNAME }}
        SMTP_PASSWORD: ${{ secrets.SMTP_PASSWORD }}
        WEBHOOK_CALLBACK_URL: ${{ secrets.WEBHOOK_CALLBACK_URL }}
        SEASONED_HAND_PHASE2_SMOKE: "1"
      run: |
        cargo test -p seasoned-hand-server --test phase2_webhook_roundtrip -- --ignored --nocapture
```

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-server --test phase2_live_overnight       # skipped without env
cargo test -p seasoned-hand-server --test phase2_webhook_roundtrip    # skipped without env
./scripts/spec-check.sh
```

Manual:
```bash
gh workflow run phase2-live-overnight
gh workflow run phase2-live-webhook-roundtrip
```

---

## Files changed

- `crates/seasoned-hand-server/tests/phase2_live_overnight.rs` (new)
- `crates/seasoned-hand-server/tests/phase2_webhook_roundtrip.rs` (new)
- `.github/workflows/ci.yml` (modify — add 2 new
  workflow_dispatch jobs)

---

## Spec references

- `/specs/phase-2/architecture.md` §11 ("E2E live-LLM workflow_dispatch")

---

## Commit message

```
test(phase-2): story 2.26 - Phase 2 live-LLM workflow_dispatch jobs

Two new workflow_dispatch CI jobs against real Bifrost + secrets:

- phase2-live-overnight: full "Do this overnight" flow. Fixed test
  brief produces summary.docx from git log; asserts completion +
  TaskComplete verdict pass + valid manifest. 30-min timeout,
  $1.50 cost cap.
- phase2-live-webhook-roundtrip: webhook intake → email delivery.
  Asserts the webhook callback URL receives the deliverable POST.

Both gated on ANTHROPIC_API_KEY + OPENAI_API_KEY (+ SMTP secrets +
WEBHOOK_CALLBACK_URL for the roundtrip job). Neither in default ci
matrix.

refs: /specs/phase-2/stories/story-2.26.md
```

---

## Notes for next story (2.27)

E2E + live-smoke jobs are in. 2.27 closes the phase: retrospective,
DEBT audit, status flips, BASELINE/CHANGELOG.
