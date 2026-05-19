# Story 2.3 — V007 + V008 + V009 migrations + remaining stores

> **Status**: done (with iter-N revisions — see "Phase 4 manageability hardening" note below)
> **Estimated**: 3 hours
> **Dependencies**: 2.2
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §3 (V007 / V008 / V009)

---

## Goal

Land every remaining schema migration Phase 2 needs PLUS the
corresponding rusqlite-backed stores. `Deliverable`, `IntakeEvent`,
`DeliveryEvent`, `NotificationsSent` become first-class persisted
entities. `Skill` and `Playbook` tables ship with the schema but no
write paths (Phase 3 populates them).

Also: AppState gains all new stores as `Arc<...>` fields. This is the
one consolidated story for the wiring so 2.5 / 2.9-2.13 / 2.14 / 2.15
can each focus on behavior rather than plumbing.

## Acceptance criteria

- [ ] `V007__phase2_deliverables.sql` creates `deliverables` per
      architecture §3 V007 verbatim.
- [ ] `V008__phase2_intake_delivery_notifications.sql` creates
      `intake_events`, `delivery_events`, `notifications_sent`. UNIQUE
      constraint `(channel, intake_id)` on `intake_events`.
- [ ] `V009__phase2_skills_playbooks.sql` creates `skills` +
      `playbooks` reservation tables. Phase 2 logic does NOT write.
- [ ] `DeliverableStore`: `insert / get / list_by_task /
      attach_provenance / mark_delivered`.
- [ ] `IntakeEventStore`: `insert / get_by_intake_id /
      link_to_task / list_by_channel`.
- [ ] `DeliveryEventStore`: `insert / list_by_task /
      list_by_deliverable`.
- [ ] `NotificationsSentStore`: `insert / list_by_task`.
- [ ] `SkillStore` + `PlaybookStore`: empty types with `new(pool)` +
      `pool_for_test` only. Phase 3 fills.
- [ ] `AppState` (server) gains `Arc<ProjectStore>`, `Arc<TaskStore>`,
      `Arc<DeliverableStore>`, `Arc<IntakeEventStore>`,
      `Arc<DeliveryEventStore>`, `Arc<NotificationsSentStore>`,
      `Arc<SkillStore>`, `Arc<PlaybookStore>`.
- [ ] All SQL parameterized.
- [ ] Unit tests: round-trip per store (`*_store_crud`), UNIQUE
      enforcement on `intake_events.(channel, intake_id)`.

## Non-goals

- Channel framework (story 2.4).
- IntakeRouter / DeliveryRouter behavior (story 2.5).
- Provenance manifest builder (story 2.15 — this story only persists
  the manifest column as TEXT; building the JSON is 2.15's job).

---

## Implementation steps

### 1. Migrations

V007, V008, V009 SQL verbatim from architecture §3. Same pattern as
V004/V005 (refinery picks up).

### 2. Module layout

```
crates/seasoned-hand-core/src/deliverable/
  mod.rs
  store.rs
  tests.rs

crates/seasoned-hand-core/src/intake/
  mod.rs
  store.rs         ← IntakeEventStore
  events.rs        ← IntakeEvent + DeliveryTarget types (referenced by 2.4)
  tests.rs

crates/seasoned-hand-core/src/delivery/
  mod.rs
  store.rs         ← DeliveryEventStore
  tests.rs

crates/seasoned-hand-core/src/notify/
  mod.rs
  store.rs         ← NotificationsSentStore
  tests.rs

crates/seasoned-hand-core/src/skill/
  mod.rs           ← SkillStore + PlaybookStore (empty Phase 2)
  tests.rs
```

### 3. AppState wiring

Add the 8 new `Arc<...>` fields to `AppState`. Initialize in
`AppState::new`. No HTTP routes yet (those land in 2.5 + 2.10 / 2.15
/ 2.22).

### 4. Tests

Each store gets a round-trip test in its `tests.rs`. The UNIQUE
constraint test inserts twice with the same `(channel, intake_id)`
and asserts the second insert errors.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core deliverable:: intake:: delivery:: notify:: skill::
cargo test -p seasoned-hand-server --lib
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/migrations/V007__phase2_deliverables.sql` (new)
- `crates/seasoned-hand-core/migrations/V008__phase2_intake_delivery_notifications.sql` (new)
- `crates/seasoned-hand-core/migrations/V009__phase2_skills_playbooks.sql` (new)
- `crates/seasoned-hand-core/src/deliverable/{mod,store,tests}.rs` (new)
- `crates/seasoned-hand-core/src/intake/{mod,store,events,tests}.rs` (new)
- `crates/seasoned-hand-core/src/delivery/{mod,store,tests}.rs` (new)
- `crates/seasoned-hand-core/src/notify/{mod,store,tests}.rs` (new — extends Phase 1 notify scaffolding if present, otherwise creates)
- `crates/seasoned-hand-core/src/skill/{mod,tests}.rs` (new)
- `crates/seasoned-hand-core/src/lib.rs` (modify — pub mod declarations)
- `crates/seasoned-hand-server/src/lib.rs` (modify — AppState gains 8 store fields)

---

## Spec references

- `/specs/phase-2/architecture.md` §3 (V007 / V008 / V009),
  §2.3 (Deliverable shape), §2.8 (IntakeEvent shape), §2.9
  (DeliveryEvent / Receipt), §2.7 (Channel framework — relevant for
  IntakeEvent type but trait surface is 2.4)

---

## Commit message

```
feat(phase-2): story 2.3 - V007/V008/V009 + Deliverable/Intake/Delivery/Notify/Skill stores

- V007: deliverables table (rendered artifact + source + provenance_manifest
  JSON column).
- V008: intake_events + delivery_events + notifications_sent.
  UNIQUE (channel, intake_id) on intake_events.
- V009: skills + playbooks reservation tables (empty in Phase 2;
  Phase 3 Curator populates).
- 6 new stores: DeliverableStore, IntakeEventStore, DeliveryEventStore,
  NotificationsSentStore, SkillStore, PlaybookStore. CRUD + pagination
  + filter where useful.
- AppState gains 8 store Arcs (incl. ProjectStore/TaskStore from
  story 2.2). No HTTP routes yet — those land in 2.5 / 2.10 / 2.15 /
  2.22.

refs: /specs/phase-2/stories/story-2.3.md
```

---

## Notes for next story (2.4)

All schema is in place. 2.4 introduces the Channel framework (three
role traits + ChannelRegistration builder + ChannelRegistry).
IntakeEvent + DeliveryTarget types from 2.3's `intake::events` module
get re-used.

## Phase 4 manageability hardening (post-execution amendment, commit `e004b2d`)

Phase 4 iter-1 manageability hardening removed two pieces of the
original 2.3 scope that turned out to be over-built:

- `SkillStore` + `PlaybookStore` (Rust types) deleted. The V009 schema
  tables (`skills`, `playbooks`) are still present and in active use,
  but the empty wrapper structs were never written through —
  Phase 3 + 4 used `crate::playbooks::*` and direct `DbPool` access
  instead. The reservation handles were pure scaffolding.
- `AppState::skills` + `AppState::playbooks` fields dropped along
  with the constructor calls. No public API broke (verified by grep
  across all crates: zero external consumers ever read either field).

The spec items listed under "Acceptance criteria" remain accurate
for the 2.3 commit (`2c36eae`) — this amendment is purely historical:
the over-built pieces were removed once Phase 4 made it clear they
were never going to be used.

Reference: `/specs/phase-2/architecture.md` §2.12 (closure note).
