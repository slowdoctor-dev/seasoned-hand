# Story 2.5 — IntakeRouter + DeliveryRouter + /v1/channels routes

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 2.4, 2.3
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.8, §2.9, §4

---

## Goal

The two router workers that connect the Channel framework (2.4) to the
event-stream + Task lifecycle (2.3). Plus the introspection routes
`GET /v1/channels` and `/v1/channels/:name/health` so operators (and
the CLI) can see what's registered.

## Acceptance criteria

- [ ] `intake::IntakeRouter` runs as a Tokio task:
      - receives `IntakeEvent` from the registry's shared mpsc
      - validates (non-empty `brief_input`, `channel` matches a
        registered name, `intake_id` unique per channel)
      - persists via `IntakeEventStore::insert`
      - creates a Task via `TaskStore::insert` in `drafted` status,
        wiring `intake.task_id` afterwards via
        `IntakeEventStore::link_to_task`
      - spawns the Initializer (legacy 1.4 entry point; the
        confirmation gate from 2.8 lands later)
      - on validation failure: `intake_rejected{reason}` Misc + 4xx
        back to the source channel (where applicable)
- [ ] `delivery::DeliveryRouter` exposes `deliver_task(task_id)`:
      - looks up the Deliverable for that task
      - resolves `reply_target` (task's override or default = intake's
        reply_target)
      - calls the matching `DeliverySink` from the registry
      - persists `DeliveryEvent { ok, external_id, error }`
      - on `ChannelError::Http(5xx)` or `Transport`: 1 retry after 30 s
      - on any other error: no retry; emit Misc `delivery_failed`
- [ ] HTTP routes:
      - `GET /v1/channels` → `{ name, capabilities: ["intake",
        "delivery", "notify"] }[]` (capabilities reflects which role
        traits the channel implements per registry introspection)
      - `GET /v1/channels/:name/health` → per-role health probe
      - `POST /v1/channels/:name/test?role=intake|delivery|notify` →
        synthetic round-trip (calls a `dry-run` mode per role; Phase 2
        accepts a stub that just returns 200 OK if the channel is
        registered and the role is impl'd)
- [ ] All HTTP routes use the shared `RouteOutcome<T>` from the Phase 1
      simplicity pass.
- [ ] Unit tests: `intake_router_persists_and_creates_task`,
      `intake_router_rejects_duplicate_intake_id`,
      `delivery_router_dispatches_to_correct_channel`,
      `delivery_router_retries_5xx_once`,
      `delivery_router_no_retry_on_decode_error`,
      `get_v1_channels_lists_capabilities`.

## Non-goals

- Concrete channel impls (stories 2.9-2.13)
- Webhook intake HTTP route `POST /v1/intake/webhook` — lands with
  WebhookChannel in story 2.10
- Briefing confirmation gate (story 2.8 wires this into Initializer
  spawning)

---

## Implementation steps

### 1. IntakeRouter

```
crates/seasoned-hand-core/src/intake/
  router.rs      ← IntakeRouter struct + run() + handle_event()
```

```rust
pub struct IntakeRouter {
    intake_store: Arc<IntakeEventStore>,
    task_store: Arc<TaskStore>,
    project_store: Arc<ProjectStore>,
    events: Arc<SqliteEventStore>,
    initializer_handle: Arc<InitializerSpawn>,  // Phase 1 1.4 spawn surface
}

impl IntakeRouter {
    pub async fn run(
        &self,
        mut rx: mpsc::Receiver<IntakeEvent>,
        shutdown: CancellationToken,
    ) -> Result<(), IntakeError> { ... }
}
```

Default project: if `IntakeEvent.metadata.project_id` is absent, look
up (or create) an `Inbox` project for the tenant. This is the
"backward compat" path for legacy `task_create { input }`.

### 2. DeliveryRouter

```
crates/seasoned-hand-core/src/delivery/router.rs
```

```rust
pub struct DeliveryRouter {
    registry: Arc<ChannelRegistry>,
    delivery_store: Arc<DeliveryEventStore>,
    deliverable_store: Arc<DeliverableStore>,
    intake_store: Arc<IntakeEventStore>,
    events: Arc<SqliteEventStore>,
    task_store: Arc<TaskStore>,
}

impl DeliveryRouter {
    pub async fn deliver_task(&self, task_id: &str) -> Result<(), DeliveryError> { ... }
}
```

Retry logic: 1 retry after 30 s for `ChannelError::Http(code)` with
500 ≤ code < 600 OR `ChannelError::Transport(_)`. Other errors are
terminal.

### 3. HTTP routes

Add to server's router builder. New handlers in
`seasoned-hand-server/src/lib.rs` (or split into `channels.rs` if the
file is getting long). All use `render_outcome` (Phase 1 simplicity
M7).

### 4. AppState wiring

`AppState` gains `Arc<IntakeRouter>` + `Arc<DeliveryRouter>` +
`Arc<ChannelRegistry>` (the registry is constructed empty at boot;
stories 2.9-2.13 register channels into it).

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core intake::router delivery::router
cargo test -p seasoned-hand-server --lib
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/intake/router.rs` (new)
- `crates/seasoned-hand-core/src/intake/mod.rs` (modify — re-export)
- `crates/seasoned-hand-core/src/delivery/router.rs` (new)
- `crates/seasoned-hand-core/src/delivery/mod.rs` (modify — re-export)
- `crates/seasoned-hand-server/src/lib.rs` (modify — AppState +
  3 channel HTTP routes)

---

## Spec references

- `/specs/phase-2/architecture.md` §2.8 (Intake protocol),
  §2.9 (Delivery protocol), §4 (API surface)

---

## Commit message

```
feat(phase-2): story 2.5 - IntakeRouter + DeliveryRouter + /v1/channels

- IntakeRouter consumes IntakeEvents from the registry's mpsc, persists
  via IntakeEventStore, creates a Task in drafted state, spawns
  Initializer. Validates: non-empty brief, registered channel,
  unique (channel, intake_id).
- DeliveryRouter::deliver_task(task_id) looks up the channel from
  task.reply_target, calls DeliverySink::deliver, persists
  DeliveryEvent. 5xx + Transport errors get 1 retry after 30 s;
  others are terminal.
- HTTP: GET /v1/channels + /v1/channels/:name/health +
  POST /v1/channels/:name/test (role-scoped). Uses shared
  RouteOutcome.
- AppState gains IntakeRouter, DeliveryRouter, ChannelRegistry Arcs.
- 6 unit tests.

refs: /specs/phase-2/stories/story-2.5.md
```

---

## Notes for next story (2.6)

Routers are in. 2.6 lands the sandbox-side renderer toolchain so
`task_deliver` (story 2.14) can produce real docx/pptx/xlsx. After 2.5
+ 2.6 land, the Channel impl stories (2.9-2.13) can each run
independently — each is a one-struct one-commit story.
