# Story 2.13 — CliChannel (process intake + stdout delivery)

> **Status**: ready
> **Estimated**: 1.5 hours
> **Dependencies**: 2.4
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.7, §2.10 (CLI)

---

## Goal

The fifth and last Phase-2 channel. `CliChannel` lets the CLI
process (`seasoned-hand task new "..."`) submit work and the result
flow back through the same CLI invocation. Implements IntakeProvider
+ DeliverySink; no NotifySink (terminal push is awkward — users use
ntfy/email for that).

## Acceptance criteria

- [ ] `seasoned_hand_core::channel::cli::CliChannel` struct holds:
      `pending: Arc<DashMap<String, oneshot::Sender<Deliverable>>>` —
      maps `intake_id` to a one-shot receiver the CLI invocation
      blocks on.
- [ ] **IntakeProvider impl**: `run()` is a no-op (intake source is
      external — CLI process calls `intake_router.push` directly via
      the existing HTTP surface OR an internal in-process channel).
- [ ] CLI's `task new "<brief>"` (story 2.21):
      - constructs an `IntakeEvent { channel: "cli", intake_id:
        format!("cli:{pid}"), brief_input, reply_target: Some({channel:
        "cli", target_ref: format!("intake:{intake_id}")}), metadata:
        { pid, cwd, user } }`
      - registers a `oneshot::Sender` in the `pending` map keyed by
        `intake_id`
      - pushes the IntakeEvent (via local in-process function call
        for the CLI-as-IntakeProvider path; OR via HTTP if `task new`
        is delegating to a remote server — both paths work because
        the CliChannel registration is local to the in-process
        invocation)
      - blocks on the `oneshot::Receiver` until the deliverable lands
- [ ] **DeliverySink impl**: `deliver(target, deliverable)`:
      - Parses `target.target_ref` (`"intake:<intake_id>"`)
      - Looks up `pending` map; if found, sends Deliverable through
        the `oneshot::Sender` → CLI invocation returns; if not found
        (CLI process exited or `--detach`), writes a fallback file to
        `~/.seasoned-hand/deliverables/<deliverable_id>.<ext>` and
        prints a note to stderr-when-running-server.
- [ ] `--detach` flag on `task new` skips registering the `oneshot`
      Sender and returns the `task_id` immediately. The deliverable
      file is the only way to read the result later (via `seasoned-hand
      task deliverable ID`).
- [ ] `--open` flag (with default-blocking, no `--detach`) shells out
      to the OS open command (`open` macOS, `xdg-open` Linux) after
      receiving the Deliverable, on the rendered file path.
- [ ] Registered: `CliChannel` registered at `AppState::new` when the
      CLI subcommand is the one launching the server (i.e., the in-
      process case). For headless server runs (`seasoned-hand server`),
      CliChannel is NOT registered — `task new` from a remote machine
      would route via WebhookChannel instead.
- [ ] Unit tests:
      - `cli_channel_intake_and_delivery_roundtrip` (in-process
        smoke; uses tokio runtime)
      - `cli_channel_detach_skips_oneshot`
      - `cli_channel_fallback_to_file_when_pending_missing`

## Non-goals

- CLI binary itself (story 2.21).
- TUI / progress rendering during the blocking wait (stretch — Phase
  2 ships a simple `Working on task ...` spinner only).

---

## Implementation steps

### 1. Module

```
crates/seasoned-hand-core/src/channel/cli.rs
```

Single file — channel is small.

### 2. In-process intake

`CliChannel::submit(brief: String, project_id: Option<String>) ->
oneshot::Receiver<Deliverable>` is the entry point the CLI calls
when running in-process. It constructs the IntakeEvent + registers
the oneshot sender + pushes through IntakeRouter, returns the
receiver.

### 3. Fallback path

When `deliver` fires but `pending.get(intake_id)` is None (CLI exited
or `--detach`), write to
`~/.seasoned-hand/deliverables/<deliverable_id>.<ext>` using `dirs`
crate (or `home` crate) for cross-platform home-dir resolution. Log
the path so server-side observers can correlate.

### 4. Tests

In-process round-trip: register oneshot → push IntakeEvent → simulate
Worker run with a fake Deliverable → call `deliver` → assert receiver
gets the deliverable.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core channel::cli
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/channel/cli.rs` (new)
- `crates/seasoned-hand-core/src/channel/mod.rs` (modify)
- `crates/seasoned-hand-server/src/lib.rs` (modify — register CliChannel
  in the in-process path; skip in headless server path)
- Possibly `crates/seasoned-hand-core/Cargo.toml` — add `dirs` if not
  already a dep

---

## Spec references

- `/specs/phase-2/architecture.md` §2.7, §2.10 (CLI)

---

## Commit message

```
feat(phase-2): story 2.13 - CliChannel (process intake + stdout delivery)

The fifth Phase-2 channel.

- CliChannel implements IntakeProvider (no-op run; intake submitted
  via in-process submit() entry point) and DeliverySink (oneshot
  channel delivery to the blocking CLI invocation, with fallback to
  ~/.seasoned-hand/deliverables/ when the CLI has exited / detached).
- No NotifySink — terminal push is awkward; ntfy/email handle that.
- --detach flag on `task new` skips registering the oneshot;
  --open shells to the OS open command on the rendered file.
- Registered only in the in-process CLI path; remote `task new` flows
  via WebhookChannel.
- 3 unit tests.

refs: /specs/phase-2/stories/story-2.13.md
```

---

## Notes for next story (2.14)

All 5 Phase-2 channels are in. 2.14 wires the `task_deliver` LLM tool
+ RendererDispatcher into the Worker mode. After 2.14, end-to-end
"submit brief → run → produce deliverable" works for any channel.
