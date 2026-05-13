# Story 2.8 — Initializer::run_with_confirmation (Briefing + confirm gate)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 2.7
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.2, §8 "Briefing confirmation timeout"

---

## Goal

Extend the Phase 1 story-1.4 `Initializer` with a confirm gate.
Before the Plan Manager seeds (and before the Worker loop starts),
the user sees a `Briefing` event in their channel and can confirm /
edit / cancel. Auto-confirm after 5 minutes (configurable). Keeps the
legacy `Initializer::run` for callers that explicitly opt out.

## Acceptance criteria

- [ ] New method `Initializer::run_with_confirmation(session_id,
      task_id, raw_input, confirm_timeout: Duration, recv_user_response:
      mpsc::Receiver<UserResponse>) -> Result<RunOutcome, InitError>`.
      Existing `run(session_id, raw_input)` stays for legacy callers
      (Phase 1 tests + WS legacy path).
- [ ] Emits `Misc{kind:"briefing_pending", briefing_call_id, task_id}` +
      `Misc{kind:"briefing", briefing_call_id, brief:Brief}` (the
      structured Brief from 2.7).
- [ ] Waits on `recv_user_response` for a matching
      `in_reply_to_call_id == briefing_call_id` with `action: "confirm"
      | "edit" | "cancel"`. Channel-agnostic (any IntakeProvider's reply
      flows here).
- [ ] On `confirm`: PlanManager seeds with `brief.phases`, task status
      → `confirmed → running`, returns `RunOutcome::Started`.
- [ ] On `edit { edits: PartialBrief }`: applies edits, re-validates,
      re-emits Briefing event with a NEW `briefing_call_id`, loops back
      to wait. Track `edits_applied` counter in the manifest (story 2.15).
- [ ] On `cancel`: task status → `cancelled`, emits `task_state{ to:
      "cancelled", reason: "user_cancelled" }` Misc, returns
      `RunOutcome::Cancelled`.
- [ ] On timeout: emits `Misc{kind:"briefing_auto_confirmed",
      briefing_call_id}`, proceeds as if confirm.
- [ ] `briefing_require_confirm: true` (env or `RunConfig`) disables
      auto-confirm — `confirm_timeout` is ignored and the wait is
      unbounded.
- [ ] Unit tests:
      - `briefing_confirm_path_starts_run`
      - `briefing_edit_emits_new_briefing` (asserts new `briefing_call_id`)
      - `briefing_cancel_transitions_to_cancelled`
      - `briefing_auto_confirm_after_timeout`
      - `briefing_require_confirm_disables_auto`
      - `briefing_caps_edit_cycles_at_5` (rejects with `BriefError::TooManyEdits`
        after 5 — prevents infinite edit loops; new soft cap in this story).

## Non-goals

- The actual WS `briefing_confirm` cmd → `UserResponse` plumbing —
  that's a Phase 0 surface (`user_response` cmd) extended in story
  2.10 (WebhookChannel handles the HTTP variant) and 2.23 (frontend
  emits the cmd). 2.8 only wires the Initializer side of the
  protocol.
- Channel-specific delivery of the Briefing event — IntakeRouter
  (story 2.5) handles forwarding via WS / webhook / email channels
  that came in 2.9-2.13.

---

## Implementation steps

### 1. Extend Initializer

```
crates/seasoned-hand-core/src/agent/init/mod.rs
```

Add the new `run_with_confirmation` method alongside `run`. Share the
brief-parse + plan-seed helpers between them.

### 2. Confirm receiver wiring

`Initializer` takes an `mpsc::Receiver<UserResponse>` per task instance.
The router (story 2.5) constructs the channel pair per task and hands
the receiver to Initializer, the sender to the WS handler. The WS
handler routes `briefing_confirm` cmds (cmd matches existing
`user_response` shape per architecture §4) into the per-task sender.

### 3. Timeout via tokio::select!

```rust
let outcome = tokio::select! {
    biased;
    user_resp = recv.recv() => handle_response(user_resp),
    _ = tokio::time::sleep(confirm_timeout), if !require_confirm
       => Outcome::AutoConfirmed,
};
```

### 4. Edit cycle cap

`tasks.brief` carries `edits_applied` in its JSON value (stored on
update). After 5 edits, return `BriefError::TooManyEdits`. Surfaces in
the UI as "please cancel and restart with a clearer brief".

### 5. Tests

In-module mock `mpsc` sender to inject confirm/edit/cancel actions.
`tokio::time::pause()` for the auto-confirm test.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core agent::init::briefing
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/agent/init/mod.rs` (modify — add
  `run_with_confirmation`)
- `crates/seasoned-hand-core/src/agent/init/briefing.rs` (new — the
  emit-and-wait loop)
- `crates/seasoned-hand-core/src/agent/init/tests.rs` (new or modify
  — append 6 tests)

---

## Spec references

- `/specs/phase-2/architecture.md` §2.2 (Briefing protocol), §8
  (timeout failure mode), §12 q1 (auto-confirm default)

---

## Commit message

```
feat(phase-2): story 2.8 - Initializer::run_with_confirmation (Briefing + confirm gate)

- New Initializer entry point waits for user confirmation via
  user_response WS verb. 5-min auto-confirm default; configurable;
  briefing_require_confirm: true disables.
- Action paths: confirm → seed Plan + transition to running; edit →
  re-emit Briefing with new call_id (capped at 5 cycles); cancel →
  cancelled state. Auto-confirm logs Misc briefing_auto_confirmed.
- Emits Misc briefing_pending + briefing event. Channel-agnostic — the
  Briefing event flows through whatever channel originated the intake.
- 6 unit tests (incl. tokio::time::pause for auto-confirm).
- Legacy Initializer::run kept for Phase 1 tests + legacy task_create
  WS path.

refs: /specs/phase-2/stories/story-2.8.md
```

---

## Notes for next story (2.9)

Brief + confirm gate are in. Stories 2.9-2.13 ship the five Phase-2
channels in parallel — each is one struct, one commit, one PR.
ChatChannel (2.9) is the smallest, wrapping the existing WS as the
default IntakeProvider + DeliverySink. Codex / Claude can pair on
2.9-2.13 with no serialization risk.
