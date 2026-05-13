# Story 2.4 — Channel framework (traits + ChannelRegistration + ChannelRegistry)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 2.1
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.7

---

## Goal

Land the **OS-shape keystone** of Phase 2: the three role traits
(`IntakeProvider`, `DeliverySink`, `NotifySink`), the `ChannelRegistration`
builder, the `ChannelRegistry` that holds registered channels and
exposes routing-friendly views. No concrete channel implementations in
this story — stories 2.9–2.13 ship the five Phase-2 channels each as
one struct with 1-3 trait impls.

## Acceptance criteria

- [ ] New crate-internal module `seasoned-hand-core::channel`.
- [ ] Traits (all `Send + Sync`, all `#[async_trait]`):
      - `IntakeProvider` with `fn name() -> &'static str` and
        `async fn run(&self, sink, shutdown) -> Result<(), ChannelError>`
      - `DeliverySink` with `name` and
        `async fn deliver(&self, target, deliverable) -> Result<DeliveryReceipt, ChannelError>`
      - `NotifySink` with `name` and
        `async fn notify(&self, target, event) -> Result<NotifyReceipt, ChannelError>`
- [ ] `ChannelError` enum (via `thiserror`): `Http(String)`,
      `Transport(String)`, `Decode(String)`, `RemoteRejected(StatusCode? + message)`,
      `Cancelled`, `Internal(String)`. Distinct variants the retry
      logic can branch on.
- [ ] `ChannelRegistration` builder: `new(name)`, `with_intake(arc)`,
      `with_delivery(arc)`, `with_notify(arc)`. Methods consume `Arc<dyn
      Role>` not `Arc<C>` so one concrete `C: IntakeProvider +
      DeliverySink + NotifySink` clones its Arc into each slot.
- [ ] `ChannelRegistry`:
      - `register(ChannelRegistration)` — stores by name.
      - `iter_intake() / iter_delivery() / iter_notify()` — returns
        only channels with the matching role populated.
      - `get_intake(name) / get_delivery(name) / get_notify(name)` —
        lookup by name; returns `None` if unregistered OR if the
        channel doesn't implement that role.
      - `health()` — list `{ name, capabilities: Vec<&'static str> }`.
- [ ] Spawn the per-channel intake task via `ChannelRegistry::spawn_intakes(shutdown)`
      which iterates `iter_intake()`, creates an mpsc sink per channel,
      and spawns each in a Tokio task. Returns one `JoinHandle` per
      spawned channel.
- [ ] Unit tests: `registry_roundtrip_three_roles`,
      `registry_intake_only_channel`, `registry_lookup_returns_none_for_missing_role`,
      `registry_iter_intake_yields_only_intake_channels`,
      `spawn_intakes_returns_one_handle_per_intake_provider`.

## Non-goals

- IntakeRouter / DeliveryRouter — story 2.5.
- Concrete channels (Webhook / Email / Chat / Cli / Ntfy) — stories
  2.9–2.13.
- HTTP routes for `/v1/channels` — story 2.5 (the router story bundles
  the introspection endpoints).

---

## Implementation steps

### 1. Trait definitions

```
crates/seasoned-hand-core/src/channel/
  mod.rs         ← re-exports + ChannelError + ChannelRegistration + ChannelRegistry
  intake.rs      ← IntakeProvider trait
  delivery.rs    ← DeliverySink trait + DeliveryTarget (or re-export from intake::events)
  notify.rs      ← NotifySink trait + NotifyTarget + NotifyEvent
  tests.rs
```

### 2. Registry

```rust
pub struct ChannelRegistry {
    by_name: HashMap<String, ChannelEntry>,
}

struct ChannelEntry {
    intake:   Option<Arc<dyn IntakeProvider>>,
    delivery: Option<Arc<dyn DeliverySink>>,
    notify:   Option<Arc<dyn NotifySink>>,
}

impl ChannelRegistry {
    pub fn register(&mut self, reg: ChannelRegistration) { ... }
    pub fn iter_intake(&self) -> impl Iterator<Item = (&str, &Arc<dyn IntakeProvider>)> { ... }
    pub fn iter_delivery(&self) -> impl Iterator<Item = ...> { ... }
    pub fn iter_notify(&self) -> impl Iterator<Item = ...> { ... }
    pub fn get_intake(&self, name: &str) -> Option<Arc<dyn IntakeProvider>> { ... }
    // ... etc

    pub async fn spawn_intakes(
        &self,
        sink: mpsc::Sender<IntakeEvent>,
        shutdown: CancellationToken,
    ) -> Vec<JoinHandle<Result<(), ChannelError>>> { ... }
}
```

### 3. Tests with mock channels

Provide a `TestChannel` struct in the test module that implements all
three traits with `AtomicUsize` counters. Tests register it three
different ways (full / intake-only / delivery-only) and assert the
registry routes correctly.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core channel::
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/channel/mod.rs` (new)
- `crates/seasoned-hand-core/src/channel/intake.rs` (new)
- `crates/seasoned-hand-core/src/channel/delivery.rs` (new)
- `crates/seasoned-hand-core/src/channel/notify.rs` (new)
- `crates/seasoned-hand-core/src/channel/tests.rs` (new)
- `crates/seasoned-hand-core/src/lib.rs` (modify — `pub mod channel;`)

---

## Spec references

- `/specs/phase-2/architecture.md` §2.7

---

## Commit message

```
feat(phase-2): story 2.4 - Channel framework (3 role traits + ChannelRegistry)

OS-shape keystone of Phase 2. A channel is one thing — an integration
with an external system — that may play 1-3 roles (IntakeProvider /
DeliverySink / NotifySink). Same 3 traits as v2.0's "adapter family"
but reshaped: implementations are one struct per integration, registered
once via ChannelRegistration builder.

- IntakeProvider trait: name() + async run(sink, shutdown). Long-lived
  listener.
- DeliverySink trait: name() + async deliver(target, deliverable).
  Call-on-demand.
- NotifySink trait: name() + async notify(target, event). Call-on-demand.
- ChannelError enum: Http / Transport / Decode / RemoteRejected /
  Cancelled / Internal. Each variant maps to a distinct retry policy
  in the routers (story 2.5).
- ChannelRegistration: builder with with_intake / with_delivery /
  with_notify Arc slots. One concrete struct can populate all three.
- ChannelRegistry: register / iter_role / get_role / health /
  spawn_intakes.
- 5 unit tests with a 3-role TestChannel mock.

refs: /specs/phase-2/stories/story-2.4.md
```

---

## Notes for next story (2.5)

Trait surface + registry are in. 2.5 ships the two routers that consume
from the registry: IntakeRouter (drains the intake mpsc → persists
IntakeEvent → spawns Initializer) and DeliveryRouter (looks up
DeliverySink by name → calls deliver → persists DeliveryEvent + retry
policy).
