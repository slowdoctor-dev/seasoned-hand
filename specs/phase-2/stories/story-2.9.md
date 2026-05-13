# Story 2.9 — ChatChannel (WS-backed IntakeProvider + DeliverySink)

> **Status**: ready
> **Estimated**: 1 hour
> **Dependencies**: 2.4, 2.5
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.7 (channel table), §2.9 "Chat delivery"

---

## Goal

Wrap the existing Phase 0 WebSocket surface as a `Channel` so it
participates uniformly in the registry. The chat panel becomes one of
several intake channels (alongside webhook, email, CLI) rather than a
hardcoded special path. `cmd: "task_create" { input }` becomes an
IntakeEvent labeled `channel = "chat"`.

## Acceptance criteria

- [ ] `seasoned_hand_core::channel::chat::ChatChannel` struct (in
      `channel/chat.rs`). Holds `Arc<SqliteEventStore>` (to emit
      Deliverable events on the WS).
- [ ] `impl IntakeProvider for ChatChannel`: `run()` is a thin
      no-op (returns `Ok(())` immediately) — the real intake source is
      the WS server, which converts incoming `task_create` cmds into
      `IntakeEvent { channel: "chat", ... }` and pushes them into the
      registry's mpsc directly. This story's `run()` exists for trait
      uniformity.
- [ ] `impl DeliverySink for ChatChannel`: `deliver(target,
      deliverable)` emits a `ServerEvent` of payload kind
      `"Deliverable"` into the session's WS stream. The
      `target.target_ref` carries `"session:<session_id>"`.
- [ ] NO `NotifySink` impl on ChatChannel (chat doesn't have separate
      push-notify semantics from regular Message events; users who
      want notify use NtfyChannel / EmailChannel).
- [ ] WS server's `task_create` handler now constructs an
      `IntakeEvent` and pushes to the IntakeRouter's mpsc instead of
      calling the runner directly. The legacy direct-call path is
      removed in favor of the unified IntakeEvent flow.
- [ ] Registered into `ChannelRegistry` at `AppState::new` via:
      ```rust
      let chat = Arc::new(ChatChannel::new(state.events.clone()));
      state.channels.register(
          ChannelRegistration::new("chat")
              .with_intake(chat.clone())
              .with_delivery(chat),
      );
      ```
- [ ] Unit tests:
      - `chat_channel_deliver_emits_ws_event` (mock WS subscriber)
      - `chat_channel_no_notify_role` (registry lookup returns None
        for the notify role)
      - `ws_task_create_creates_intake_event` (existing WS test
        extended)

## Non-goals

- New WS envelope shapes — uses the existing `ServerEvent { payload }`
  envelope. The Deliverable payload type is reserved in
  `frontend/lib/ws-types.ts` (frontend story 2.22 renders it).
- Briefing UI (frontend, story 2.23).

---

## Implementation steps

### 1. Channel module

```
crates/seasoned-hand-core/src/channel/chat.rs
```

```rust
pub struct ChatChannel {
    events: Arc<SqliteEventStore>,
}

#[async_trait]
impl IntakeProvider for ChatChannel {
    fn name(&self) -> &'static str { "chat" }
    async fn run(&self, _sink, _shutdown) -> Result<(), ChannelError> {
        // WS server pushes IntakeEvents directly into the registry's
        // shared mpsc; this trait method is a no-op for uniformity.
        Ok(())
    }
}

#[async_trait]
impl DeliverySink for ChatChannel {
    fn name(&self) -> &'static str { "chat" }
    async fn deliver(&self, target, deliverable) -> Result<DeliveryReceipt, ChannelError> {
        // Emit a Message event of kind "Deliverable" with the file_ref.
        // Frontend (story 2.22) filters by kind:"Deliverable" and renders
        // as a card.
        ...
    }
}
```

### 2. WS handler refactor

`crates/seasoned-hand-server/src/ws.rs` — find the existing
`task_create` cmd handler. Replace direct `runner.run` call with:

```rust
let intake_event = IntakeEvent {
    channel: "chat".into(),
    intake_id: format!("ws:{}", uuid),
    brief_input: cmd.input.clone(),
    reply_target: Some(DeliveryTarget {
        channel: "chat".into(),
        target_ref: format!("session:{}", session_id),
        metadata: json!({}),
    }),
    received_at: now_unix(),
    metadata: json!({"ws_msg_id": msg_id}),
    tenant_id: None,
};
state.intake_router.push(intake_event).await?;
```

### 3. Tests

WS test that submits `task_create` and asserts an `intake_events` row
is created + the legacy `task_create` ack still returns the session_id.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core channel::chat
cargo test -p seasoned-hand-server ws::
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/channel/chat.rs` (new)
- `crates/seasoned-hand-core/src/channel/mod.rs` (modify — `pub mod chat;`)
- `crates/seasoned-hand-server/src/ws.rs` (modify — `task_create`
  handler uses IntakeRouter)
- `crates/seasoned-hand-server/src/lib.rs` (modify — register ChatChannel
  at boot)

---

## Spec references

- `/specs/phase-2/architecture.md` §2.7 (channel matrix),
  §2.8 (intake protocol)

---

## Commit message

```
feat(phase-2): story 2.9 - ChatChannel wraps existing WS as a Channel

- ChatChannel implements IntakeProvider (no-op trait method; WS server
  pushes IntakeEvents directly) and DeliverySink (emits ServerEvent of
  kind Deliverable into session WS stream).
- No NotifySink impl — chat has no separate push-notify semantics
  distinct from regular messages.
- WS server's task_create handler now constructs an IntakeEvent
  labeled channel="chat" and pushes through IntakeRouter. Legacy
  direct-runner-call removed; all intake flows through the unified
  channel registry path.
- 3 unit tests.

refs: /specs/phase-2/stories/story-2.9.md
```

---

## Notes for next story (2.10)

Chat is the simplest channel — one role impl is a no-op. Webhook
(2.10) is more substantive: real HTTP intake endpoint, real POST
delivery, real POST notify. Same one-struct pattern, three trait
impls.
