# Story 2.25 — Phase 2 E2E (deterministic 50-step + briefing + email roundtrip)

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 2.2-2.20
> **Phase**: 2
> **Type**: test
> **Reads first**: `/specs/phase-2/architecture.md` §11 "Acceptance gate" + Phase 1 1.20 (template)

---

## Goal

Acceptance gate for Phase 2: a deterministic test on the default
`cargo test --workspace` path that drives the full "Do this overnight"
flow end-to-end. Mirrors Phase 1 story 1.20's `phase1_stable_50step`
pattern but adds briefing, durable pause+resume, email-channel
intake+delivery roundtrip, and provenance manifest assertions.

## Acceptance criteria

- [ ] `crates/seasoned-hand-server/tests/phase2_overnight.rs` runs
      on the default `cargo test --workspace` path (NOT `#[ignore]`).
- [ ] Wiremock'd Bifrost scripts a deterministic ≥50-step task with
      ~3 phases. Same shape as Phase 1's `phase1_stable_50step`.
- [ ] Wiremock'd SMTP (lettre `StubTransport`) for email delivery.
      Wiremock'd IMAP via `async-imap`'s test fixture for email intake.
- [ ] Flow assertions:
      - Email arrives on the IMAP server (test fixture); IntakeEvent
        created; Task created in `drafted`.
      - Initializer emits Briefing; auto-confirm fires after the test
        clock advances 5 min (via `tokio::time::pause`); Task → `running`.
      - Worker runs ≥50 tool calls without `stuck_terminate`,
        `max_steps_reached`, or `cost_cap` Misc.
      - Mid-run, simulate `task_pause { durable: true }`. Kill
        sandbox handle (via `insert_handle_for_test` with empty handle).
        Call `task_resume`. Assert: new Session row, replayed Plan,
        replayed feature-list, task back to `running`.
      - Worker calls `task_deliver` with a `.docx` target. Renderer
        wiremock'd to return canned PNG-like bytes representing a
        rendered docx.
      - Deliverable persisted; provenance manifest contains all
        required fields (golden-file diff).
      - Email delivery: `StubTransport` captured a reply email; its
        `In-Reply-To` matches the original Message-ID; attachment
        filename is the `target_filename`.
      - Verifier verdict: exactly one `verifier_verdict` Misc with
        `trigger_kind == "TaskComplete"` and `verdict == "pass"`.
- [ ] Wall-clock budget NOT asserted on the default path (wiremock
      makes the test fast). When `SEASONED_HAND_PHASE2_SMOKE=1`,
      assert wall < 600s (mirrors Phase 1 pattern).
- [ ] Test runs in <30s on the default path.

## Non-goals

- Live-LLM round-trip (story 2.26).
- Multi-channel branching (test focuses on the email channel only;
  webhook + chat + CLI flows tested in their own per-channel tests).
- Frontend Playwright coverage (story 2.24).

---

## Implementation steps

### 1. Test scaffold

Mirror `tests/phase1_stable_50step.rs` structure. Bring up:
- `db::open(":memory:")` + migrations
- `pubsub::RedisPool::new("redis://127.0.0.1:6")` (the existing
  unreachable placeholder; live-Redis paths skip cleanly)
- `SandboxClient` with `insert_handle_for_test`
- Wiremock for Bifrost (planner + main + classifier slots)
- Wiremock for `/v1/shell/exec` (sandbox renderer install + git +
  Pandoc invocations)
- `lettre::transport::stub::StubTransport` for SMTP
- Synthetic IMAP fixture (canned new message via `async-imap` test
  server)

### 2. Scripted LLM responses

```rust
let llm_responses = vec![
    planner_brief_response(),                  // Initializer's brief generation
    // ~50 worker iteration responses (one tool call each)
    tool_call("file_write", ...),
    // ...
    tool_call("task_deliver", json!({
        "content": "# Summary\n\n...",
        "target_filename": "summary.docx",
        "citations": [1, 2, 3],
    })),
    // verifier verdict response
    verifier_pass_response(),
];
```

### 3. Mid-run pause+resume injection

After ~10 tool calls, the test issues a `task_pause { durable: true }`
WS cmd, drops the sandbox handle, then issues `task_resume`. Asserts
the rebuild path fires (Misc `task_resume_rebuild_required` event
present).

### 4. Provenance golden file

`tests/fixtures/phase2_overnight/expected_provenance.json` — the
expected manifest shape (with placeholders for nondeterministic IDs).
Test compares with field-by-field equality (ids omitted from compare).

### 5. Wiremock'd renderer

`/v1/shell/exec` for the Pandoc call returns canned bytes representing
a docx. Test asserts the request body contained `pandoc -f markdown
-t docx ...`.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-server --test phase2_overnight
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-server/tests/phase2_overnight.rs` (new)
- `crates/seasoned-hand-server/tests/fixtures/phase2_overnight/` (new
  — expected_provenance.json + canned LLM responses)

---

## Spec references

- `/specs/phase-2/architecture.md` §11 ("Acceptance gate")
- Phase 1 story 1.20 (`phase1_stable_50step.rs`) — template

---

## Commit message

```
test(phase-2): story 2.25 - Phase 2 deterministic E2E (overnight workflow)

Acceptance gate test for Phase 2 on the default cargo test --workspace
path. Drives the full "Do this overnight" flow:

- Email arrives via IMAP fixture → IntakeEvent → Task drafted
- Initializer emits Briefing → auto-confirms after tokio::time::pause
  advances 5 min → Task running
- Worker runs 50+ scripted tool calls without stuck/max-steps/cost
- Mid-run durable pause + sandbox-handle drop + resume + rebuild via
  event-stream replay
- task_deliver renders a .docx via wiremock'd Pandoc; Deliverable
  persisted with full provenance manifest (golden-file diff)
- Email delivery: lettre StubTransport captured reply with
  In-Reply-To + .docx attachment
- Exactly one TaskComplete verifier_verdict pass

Runs in <30s on the default path. Wall-clock budget asserted only
when SEASONED_HAND_PHASE2_SMOKE=1.

refs: /specs/phase-2/stories/story-2.25.md
```

---

## Notes for next story (2.26)

Deterministic E2E green. 2.26 wraps the same flow into a
workflow_dispatch CI job against real Bifrost + Anthropic + OpenAI
keys for `phase2-live-overnight` smoke.
