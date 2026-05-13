# Story 2.12 — NtfyChannel + NotifyWorker

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 2.4
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.7 (channel matrix), §2.9, §2.12 closeouts list

---

## Goal

Two related pieces:
1. **NtfyChannel** — notify-only channel; posts to ntfy.sh (or a
   self-hosted ntfy server).
2. **NotifyWorker** — consumes the Redis `notify_request` stream and
   dispatches to registered `NotifySink` channels per the configured
   per-trigger routing table.

Architecturally NotifyWorker is the symmetric outbound counterpart to
the IntakeRouter; it generalizes the v1.0 notify worker design to use
the v2.1 channel framework.

## Acceptance criteria

- [ ] `seasoned_hand_core::channel::ntfy::NtfyChannel` struct holds:
      `host: String` (default `https://ntfy.sh`), `http:
      reqwest::Client`. Implements **only** `NotifySink`.
- [ ] `NtfyChannel::notify(target, event)`:
      - `target.target_ref` is the ntfy topic
      - POST `{host}/{topic}` with body `payload` (the notify event
        payload, redacted per architecture §9)
      - Headers: `Title:`, `Priority:`, `Tags:` derived from
        `event.metadata`
      - Returns `DeliveryReceipt { external_id: ntfy_msg_id_from_response, ... }`
- [ ] `seasoned_hand_core::notify::NotifyWorker` runs as a Tokio
      task:
      - `XREADGROUP GROUP notify-workers <consumer> BLOCK 5000 COUNT 16
        STREAMS notify_request >` (real consumer loop, NOT the polling
        stub of Phase 1 1.9b — DEBT #15 closes for the VERIFIER worker
        in story 2.18; this worker's design ships correct from day 1).
      - Each message: parse `NotifyRequest { trigger_kind, task_id?,
        payload, target_channels: Vec<String> }`.
      - For each channel name in `target_channels`: look up
        `NotifySink` in the registry; call `notify(target, event)`;
        persist a `notifications_sent` row.
      - On `ChannelError::Http(5xx)` for webhook only: 1 retry after
        30 s. Other adapters / errors: no retry (best-effort).
      - XACK on success path AND on terminal-error path; only
        unparseable messages stay in the PEL for ops review.
- [ ] Triggers: the Phase 2 event-stream listener (extends Phase 1's
      InvalidationHook pattern) emits a `notify_request` XADD on:
      - `task_state { to: "completed" }` → `trigger_kind:
        "task_finished"`, channels from `config/notify.toml
        [trigger.task_finished].channels`
      - `task_state { to: "failed" }` → `task_failed`
      - `briefing_pending` → `briefing_pending` (opt-in)
      - `verifier_verdict { verdict: "fail" }` → `verifier_fail` (opt-in)
- [ ] Config: `config/notify.toml` (new) with `[trigger.X].channels =
      ["ntfy", "email"]`. Architecture §2.7 shape verbatim.
- [ ] Registered: `NtfyChannel` registered at boot only when
      `NTFY_TOPIC` env is set (otherwise skipped — the channel is
      optional). NotifyWorker spawned unconditionally when
      `notify_request` consumer-group can be created on the configured
      Redis (skipped on Redis unreachable, mirroring Phase 1 verifier
      worker pattern).
- [ ] Unit tests:
      - `ntfy_channel_posts_to_topic` (wiremock)
      - `ntfy_channel_sets_title_priority_tags`
      - `notify_worker_consumes_and_dispatches` (live-Redis `#[ignore]`)
      - `notify_worker_xacks_on_dispatch_error` (live-Redis `#[ignore]`)
      - `event_listener_emits_notify_request_on_task_completed`
      - `event_listener_skips_notify_for_unconfigured_trigger`

## Non-goals

- Webhook + Email notify (those are part of stories 2.10 + 2.11).
- Frontend notification surface (Phase 2 deliberately defers — users
  receive notifies via the channel, not via the in-app UI).
- Per-tenant notify config (Phase 5).

---

## Implementation steps

### 1. NtfyChannel

```
crates/seasoned-hand-core/src/channel/ntfy.rs
```

Single-file because the channel is small (notify-only).

### 2. NotifyWorker

```
crates/seasoned-hand-core/src/notify/
  worker.rs       ← consume notify_request + dispatch
  config.rs       ← read [trigger.*] from notify.toml
  listener.rs     ← event-stream hook that XADDs notify_request
  tests.rs
```

### 3. Event-stream listener

The listener subscribes to the event stream (Phase 0 pubsub) and
filters for the four trigger events listed above. For each match, it
loads the per-trigger channels list from config and XADDs one
`notify_request` per channel (NOT per-trigger — the worker iterates
channels itself, but the XADD batches them).

Wait — re-read architecture §2.7: the worker iterates `target_channels`
itself. So the listener XADDs ONE message per trigger, carrying the
channels list. Worker then fans out to each channel.

### 4. Config loader

`config/notify.toml` parsed at boot. Validation: every channel name
must be registered. If a channel referenced in config is missing from
the registry, log `notify_config_channel_missing` warning and proceed
without it.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core channel::ntfy notify::
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/channel/ntfy.rs` (new)
- `crates/seasoned-hand-core/src/channel/mod.rs` (modify)
- `crates/seasoned-hand-core/src/notify/worker.rs` (new — or modify if
  Phase 1 stubs exist)
- `crates/seasoned-hand-core/src/notify/config.rs` (new)
- `crates/seasoned-hand-core/src/notify/listener.rs` (new)
- `crates/seasoned-hand-core/src/notify/tests.rs` (new or modify)
- `config/notify.toml` (new — example config)
- `crates/seasoned-hand-server/src/lib.rs` (modify — register NtfyChannel
  when configured, spawn NotifyWorker)

---

## Spec references

- `/specs/phase-2/architecture.md` §2.7, §2.9 (delivery semantics
  echo notify semantics)

---

## Commit message

```
feat(phase-2): story 2.12 - NtfyChannel + NotifyWorker

- NtfyChannel: notify-only channel. POST to {host}/{topic} with
  payload + Title/Priority/Tags headers.
- NotifyWorker: real XREADGROUP consumer of notify_request Redis
  stream. Per-message: parse, look up NotifySink channels from
  registry, dispatch, persist notifications_sent rows.
- Event-stream listener emits notify_request XADD on task_state
  completed / failed, briefing_pending (opt-in), verifier_verdict
  fail (opt-in). Per-trigger channel routing from config/notify.toml.
- Best-effort delivery; 1 retry after 30s on webhook 5xx only;
  others terminal. XACK on success + terminal error; PEL holds only
  unparseable messages.
- 6 unit tests (3 live-Redis #[ignore]).

refs: /specs/phase-2/stories/story-2.12.md
```

---

## Notes for next story (2.13)

NtfyChannel + NotifyWorker complete the notify surface. 2.13 wraps
the CLI's input-from-process and output-to-stdout as the fifth and
final Phase-2 channel.
