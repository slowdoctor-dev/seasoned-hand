# Phase 2 — Technical Debt Ledger

> Append-only list of shortcuts, stubs, simplifications, and deferred
> work introduced during Phase 2. Same discipline as Phase 0 / Phase 1
> DEBT.md.
>
> Seeded at architecture phase boundary (2026-05-13). Items added during
> story implementation get appended below the seed block.

---

## Seed (from architecture v2.1, 2026-05-13)

### 1. WebhookChannel SSRF protection is permissive by default
- **Origin**: architecture.md §9 "Webhook delivery URL"
- **Severity**: **Medium**
- **What**: `WebhookChannel`'s `DeliverySink` impl rejects URLs that
  resolve to private / link-local / loopback addresses by default —
  but an operator allow-list bypasses the check. In Phase 2
  single-user, the operator IS the only caller, so trust is high.
- **Why**: Phase 5 multi-user makes webhook URLs user-supplied;
  attacker-controlled `reply_target.url` pointing at `http://10.0.0.1/admin`
  becomes a real SSRF.
- **Pay down**: Phase 5 tightens — webhook URLs from untrusted users
  must always resolve to public IPs; allow-list bypass requires admin
  scope.
- **Status (story 2.10)**: Default-deny posture is now implemented in
  `crates/seasoned-hand-core/src/channel/webhook/ssrf.rs` —
  `assert_public_address` resolves the URL's host and rejects any
  loopback / private / link-local / multicast / unspecified address
  (both IPv4 and IPv6) with `RemoteRejected { status: 400, message:
  "private_address_rejected" }` (terminal — the DeliveryRouter does
  not retry). Operators bypass per-CIDR via the
  `WEBHOOK_DELIVERY_ALLOWLIST` env (`10.0.0.0/8,192.168.0.0/16,...`).
  Phase 5 will gate the bypass behind admin scope; the default-deny
  contract is now locked in.

### 2. Sandbox-side renderer toolchain via startup-install
- **Origin**: architecture.md §2.3 + §5 "Sandbox-side renderer toolchain"
- **Severity**: **Low**
- **What**: Phase 2 installs Pandoc + python-pptx + openpyxl via
  `apt install -y pandoc texlive-xetex && pip install python-pptx openpyxl`
  at session-create time (~30-60 s per session). Each new sandbox
  re-installs from scratch.
- **Why**: Avoids the operational lift of forking + publishing a
  `seasoned-hand-sandbox` image in Phase 2.
- **Pay down**: Phase 4 — once the renderer set stabilizes, bake a
  pre-published `seasoned-hand-sandbox:phase-4` image with the
  toolchain. Cuts session-spawn time from 30-60 s to <5 s.

### 3. Code-as-deliverable is git-tree-only in Phase 2
- **Origin**: architecture.md §2.3 + §12 q7
- **Severity**: **Low**
- **What**: Phase 2's "code" deliverable is the sandbox git tree
  itself (operator can `git clone` post-completion). No GitHub PR
  creation, no live deployment automation. The deliverable.format =
  "code" implies "go look at the sandbox workspace".
- **Why**: Auth-dependent GitHub/GitLab channels need Phase 5
  multi-user. Live deployment via `deploy_expose_port` exists as a
  Phase 0 tool but Phase 2 doesn't wire it into the deliverable flow.
- **Pay down**: Phase 4 — `GitHubChannel` (DeliverySink: PR creation)
  + composite `{ git_sha, deploy_url }` format.

### 4. Email allow-list is operator-curated
- **Origin**: architecture.md §9 "Email intake authentication"
- **Severity**: **Low**
- **What**: `INTAKE_EMAIL_ALLOWED_SENDERS` env (regex allow-list)
  defaults to empty (deny all). Operator manually whitelists own email
  + collaborators. No discovery / invite UX in Phase 2.
- **Pay down**: Phase 5 multi-user — per-user allow-list managed via
  account settings UI.

### 5. Provenance manifest size budget = 100 KB inline
- **Origin**: architecture.md §12 q5
- **Severity**: **Low**
- **What**: Provenance manifests stored inline in
  `deliverables.provenance_manifest` (JSON TEXT column). Manifests
  exceeding 100 KB (extreme long-running tasks with thousands of
  events) spill to `/workspace/.provenance/<task_id>.json` and the
  column stores a file-ref instead.
- **Pay down**: Phase 3+ — Curator may compress old manifests; Phase 5
  may move to a dedicated provenance store.

### 6. Skill / playbook tables empty in Phase 2
- **Origin**: architecture.md §2.12 + V009
- **Severity**: **n/a** (informational)
- **What**: V009 creates `skills` + `playbooks` tables with the
  expected schema; Phase 2 logic never writes rows. Phase 3 (learning)
  populates them.
- **Why**: Reserves the slot so Phase 3 is purely logic, not
  schema migration. Forward-compat principle.
- **Pay down**: Phase 3 — implement Curator + post-task playbook
  extraction.

### 7. Verifier rollback default still opt-in (Phase 1 DEBT #3 carryover)
- **Origin**: Phase 1 DEBT #3, Phase 2 closeout decision
- **Severity**: **Medium**
- **What**: Phase 2 closeout (story 2.27) collects verifier verdict
  precision from real "Do this overnight" runs and decides whether to
  flip `checkpoint_rollback_on_verifier_fail` default from `false` to
  `true`. If precision >90%, flip; else carry into Phase 3.
- **Pay down**: This story (2.27) — data-driven decision.

### 8. CLI auth deferred to Phase 5
- **Origin**: architecture.md §9 "CLI security"
- **Severity**: **Low**
- **What**: Phase 2 CLI is unauthenticated, talks to localhost. No
  `seasoned-hand auth login` flow. Operator runs CLI on the same
  machine as the server.
- **Pay down**: Phase 5 multi-user — OAuth/JWT, per-user tokens
  managed via `~/.seasoned-hand/credentials`.

### 9. Phase 1 DEBT items NOT paid down by Phase 2
- **Origin**: architecture.md §0 + Phase 1 closeout retrospective
- **Severity**: **n/a** (informational)
- **What**: Phase 1 DEBT items intentionally NOT addressed in Phase 2:
  - **#1, #2** ARCHITECTURE.md text drift — doc-only, no urgency
  - **#3** Verifier rollback default — addressed (see item 7 above)
  - **#4** Single invalidation heuristic — Phase 4 (Curator)
  - **#5** Single verifier slot for all triggers — Phase 4
  - **#6** Egress allowlist default — Phase 5 (multi-user)
  - **#7** Diversity Injector variants Rust-const — Phase 4 (Curator
    can promote variants to DB)
  - **#8** PostBrowserAction screenshot retention — folded into
    Phase 0 DEBT #16 (workspace TTL) which Phase 2 pays down
  - **#10** Lazy evidence_event_ids resolution — Phase 5
  - **#11** Sandbox git identity hardcoded — Phase 5
  - **#12** Verifier 5xx fail-closed — revisit if real outage data
- **Pay down**: Each linked to its target phase above.

---

## Story-introduced (chronological)

### ~~10. `Deliverable` struct lives inside `channel/delivery.rs`~~ — CLOSED in story 2.3
- **Origin**: story 2.4 (`93fff98`), `crates/seasoned-hand-core/src/channel/delivery.rs`
- **Severity**: **Low**
- **What**: The `Deliverable` placeholder struct (V007 column shape)
  lives in `channel::delivery` instead of its eventual home in a
  dedicated `deliverable` module. Needed there so the
  `DeliverySink::deliver(target, deliverable)` trait signature is
  self-contained in story 2.4 (before 2.3 lands V007 +
  `DeliverableStore`).
- **Why**: Avoids forward-declaration dance; keeps 2.4 as a pure
  trait-surface story without dragging in V007 migration. Also avoids
  defining the Deliverable type in two places (channel module + 2.3's
  store module) and having to reconcile.
- **Pay down**: Story 2.3 — when `DeliverableStore` lands, decide
  whether to (a) keep `channel::Deliverable` as the canonical shape
  and have the store wrap it, or (b) move it into a top-level
  `deliverable` module and have `channel::delivery` re-export. Either
  way, the existing `Deliverable` shape is the V007 column projection,
  so no breaking change to callers.
- **Resolution (story 2.3)**: Chose **option (b)** — `Deliverable`
  now lives in `crates/seasoned-hand-core/src/deliverable/` (the
  canonical home, co-located with `DeliverableStore` + V007). The
  struct is expanded to the full V007 column projection (all 12
  columns). `channel::delivery` `pub use`s it so `DeliverySink::deliver`
  stays self-contained and 2.4 / 2.5 callers see no path change.

### ~~11. `DeliverableStore::mark_delivered` is a row-existence stub~~ — CLOSED in story 2.5
- **Origin**: story 2.3, `crates/seasoned-hand-core/src/deliverable/store.rs`
- **Severity**: **Low**
- **What**: `mark_delivered(id)` only validates the deliverable row
  exists; it has no side effect on the row itself. V007 has no
  `delivered_at` column, and the audit trail lives in V008
  `delivery_events`, so the natural per-row stamp has nowhere to land.
- **Why**: Story 2.3's scope is *plumbing*, not behavior — non-goals
  explicitly defer DeliveryRouter wiring to story 2.5 and the
  provenance manifest builder to story 2.15. The minimal honest stub
  beats inventing a column or speculatively updating the manifest
  here.
- **Pay down**: Story 2.5 (DeliveryRouter) calls `mark_delivered` as
  the existence guard before appending a `delivery_events` row; story
  2.15 owns appending to the manifest's `delivered_to[]` array via
  `attach_provenance`. No new column needed.
- **Resolution (story 2.5)**: `DeliveryRouter::deliver_task` calls
  `deliverable_store.mark_delivered(deliverable.id)` immediately
  before invoking the `DeliverySink` (see
  `crates/seasoned-hand-core/src/delivery/router.rs`). The contract
  is honoured — the guard returns `DeliverableError::NotFound` when
  the row is missing, preventing a phantom `delivery_events` row from
  being persisted against a deliverable that doesn't exist. Story
  2.15 will still own the manifest `delivered_to[]` write.

### ~~12. IntakeRouter cannot emit `intake_rejected` Misc or 4xx~~ — partially closed in story 2.10 (4xx surface lands; Misc emit still deferred)
- **Origin**: story 2.5, `crates/seasoned-hand-core/src/intake/router.rs`
- **Severity**: **Low**
- **What**: Validation failures (empty brief, unregistered channel)
  surface as `HandleOutcome::Rejected(reason)` and a
  `tracing::warn!` line — but the spec's
  `intake_rejected{reason}` Misc event and 4xx-back-to-source-channel
  responses are NOT emitted.
- **Why**: `Misc` events require an existing `sessions.id` FK target
  (see `events::sqlite::SqliteEventStore::append`'s session-existence
  guard at lines 62–71). Intake rejection happens *before* task /
  session creation, so there is no anchor to attach the event to. The
  matching 4xx response needs the `POST /v1/intake/webhook` endpoint,
  which story 2.10 (WebhookChannel) ships.
- **Pay down**: Story 2.10 — once `POST /v1/intake/webhook` exists,
  validation rejections short-circuit there with the 4xx response.
  Story 2.18+ may also introduce a per-tenant "system session" that
  pre-task Misc events can land on (defer until a second feature
  needs it; YAGNI for 2.5).
- **Resolution (story 2.10, partial)**: The webhook intake handler in
  `crates/seasoned-hand-server/src/lib.rs` (`post_intake_webhook_handler`)
  now maps `HandleOutcome::Rejected(reason)` to a `400` carrying
  `intake_rejected:<reason>` (`empty_brief` or `unknown_channel`). The
  `intake_rejected` Misc event still has no session FK to attach to,
  so the pre-task Misc emit is deferred until a system-session strategy
  exists. Email / Slack / other future intake channels will route
  rejection through the same 4xx surface.

### 13. IntakeRouter does not spawn the Initializer
- **Origin**: story 2.5, `crates/seasoned-hand-core/src/intake/router.rs`
- **Severity**: **Low**
- **What**: After persisting the intake row + creating the `drafted`
  Task + linking the two, the router *stops*. The Initializer (Phase
  1 1.4 entry point) is never invoked, so no Brief is authored and
  the Task sits in `drafted` forever in production.
- **Why**: Story 2.5's spec acceptance treats Initializer spawn as
  out-of-scope ("spawns the Initializer (legacy 1.4 entry point; the
  confirmation gate from 2.8 lands later)") — story 2.8 introduces
  the briefing-confirmation gate that owns the spawn surface.
  Coupling 2.5 to 1.4's existing async-task spawn would prematurely
  commit to a shape that 2.8 may need to refactor.
- **Pay down**: Story 2.8 wires an `InitializerSpawn` handle through
  the `IntakeRouter` constructor and invokes it from
  `handle_event(...)` after the task is created.
- **Status (story 2.8)**: **Still open.** Story 2.8 landed
  `Initializer::run_with_confirmation` (Briefing emit + confirm gate +
  edit cap + auto-confirm) as a self-contained method with 6 unit
  tests, but explicitly DID NOT wire the per-`briefing_call_id`
  sender map onto `AppState` or hook the IntakeRouter into a spawn
  path. The wiring (DashMap of `briefing_call_id →
  mpsc::Sender<UserResponse>` on AppState, IntakeRouter ctor change,
  WS `briefing_confirm` cmd handler, WS `task_create` refactor to
  drop the legacy direct-spawn shim) is the larger half of the
  blast-radius and overflowed the 2-hour solo budget. Tracked as
  story 2.8b (or rolled into 2.14 deliverable-pipeline prep).

### 14. `ProjectStore::find_or_create_inbox` has no UNIQUE backstop
- **Origin**: story 2.5, `crates/seasoned-hand-core/src/project/project.rs`
- **Severity**: **Low**
- **What**: V006's `projects` table has no `UNIQUE (tenant_id, title)`
  constraint, so two concurrent `find_or_create_inbox(...)` calls on
  the same tenant can both fall through to INSERT and create two
  `Inbox` rows. The router treats whichever the next SELECT returns
  as canonical, but the orphan stays.
- **Why**: Phase 2 is single-operator, so concurrent intake at the
  same tenant is implausible (`hand:0.1` is one human). Adding a
  UNIQUE migration just to defend against an impossible race is
  premature.
- **Pay down**: Phase 5 multi-tenant — when concurrent intake at the
  same tenant becomes realistic, ship `V0NN_unique_tenant_title.sql`
  with a one-time dedup of any existing duplicate Inbox rows.

### 15. WS `task_create` still spawns the runner directly (Phase 0/1 shim)
- **Origin**: story 2.9, `crates/seasoned-hand-server/src/ws.rs`
- **Severity**: **Low**
- **What**: The WS `task_create` handler now pushes an
  `IntakeEvent { channel: "chat", ... }` through
  `state.intake_router.handle_event(...)` (so the V008 `intake_events`
  row + drafted Task land per the §2.8 contract) — BUT it still also
  inserts a `sessions` row and spawns `AgentRunner::run` directly, the
  same as the Phase 0/1 path. The synchronous WS Ack carrying
  `session_id` is preserved so existing chat clients keep working.
- **Why**: The Initializer spawn that would turn the drafted Task into
  a running session is Phase 2 DEBT #13 (story 2.5 → 2.8). Removing
  the legacy direct-spawn before #13 closes would break the
  `task_create_returns_session_id_and_starts_runner` contract and
  leave chat-originated tasks stuck in `drafted` forever. The spec for
  2.9 (Implementation step 2 + Files-changed comment) calls for the
  full inversion; the gotchas section also explicitly authorises
  "stay scoped to 'intake row lands in DB' and explicitly punt the
  Initializer spawn with a stronger DEBT note" — this entry is that
  note.
- **Pay down**: Story 2.8 closes DEBT #13 by wiring the Initializer
  through the IntakeRouter. At the same time, the WS handler's
  `insert_session_row` + `runner.run` block is replaced by a return of
  `{task_id, briefing_call_id}` (or kept as a back-compat alias that
  shells through to the Initializer). The
  `task_create_returns_session_id_and_starts_runner` test is updated
  or split.
- **Status (story 2.8)**: **Still open** — see DEBT #13 status note
  for the same reason. Story 2.8 landed the Initializer's
  `run_with_confirmation` surface but not the WS / AppState wiring;
  the legacy direct-spawn shim still owns chat-originated tasks.

### ~~16. IntakeRouter `run` loop + shared mpsc not yet spawned in `main.rs`~~ — CLOSED in story 2.10
- **Origin**: story 2.9, `crates/seasoned-hand-server/src/main.rs`
- **Severity**: **Low**
- **What**: The architecture §2.8 contract is that all
  `IntakeProvider` impls push into a single
  `mpsc::Sender<IntakeEvent>` that `IntakeRouter::run` drains. Story
  2.9 doesn't add that boot wiring — the ChatChannel's
  `IntakeProvider::run` is a documented no-op and the WS handler calls
  `IntakeRouter::handle_event` synchronously, so there is no producer
  that needs the mpsc yet.
- **Why**: Adding the channel + spawn + shutdown cancellation now,
  with zero real long-lived intake providers, would be speculative
  plumbing. Tests would also have to spawn the router themselves
  (their `boot()` helpers use `AppState::new` directly and never call
  `main`'s wiring).
- **Pay down**: Story 2.10 (`WebhookChannel`) ships the first real
  long-lived IntakeProvider; that story:
  - adds `intake_events_tx: mpsc::Sender<IntakeEvent>` to `AppState`
    (or a similar fan-in handle),
  - spawns `IntakeRouter::run(rx, shutdown)` from `main.rs`,
  - moves the WS handler from direct `handle_event` to mpsc push so a
    single drain loop owns ordering across all intake providers.
- **Resolution (story 2.10)**: `main.rs` now creates the
  `(intake_tx, intake_rx)` pair, spawns `IntakeRouter::run(intake_rx,
  intake_shutdown)`, and calls
  `state.channels.spawn_intakes(intake_tx, intake_shutdown.clone())`
  to start every long-lived intake provider in lock-step. Both Chat
  and Webhook channels currently park their `run()` on the shutdown
  token (their intake source is external — WS for chat, the
  `POST /v1/intake/webhook` route for webhook), so the drain loop is
  idle today but ready for EmailChannel (story 2.11). The WS handler
  staying on direct `handle_event` is intentional for now: the WS Ack
  must return a synchronous `session_id`, which the mpsc push path
  can't supply without further refactor; the legacy bridge is tracked
  separately as DEBT #15 (closes when story 2.8 lands the briefing
  gate).

### 18. EmailChannel discards attachment bytes after extracting metadata
- **Origin**: story 2.11, `crates/seasoned-hand-core/src/channel/email/mod.rs`
  (`process_message`)
- **Severity**: **Medium**
- **What**: Inbound email attachments are parsed (filename, content type,
  size, sha-able bytes) and surfaced into `IntakeEvent.metadata.attachments[]`,
  but the byte payload is **not** persisted anywhere. The architecture
  §2.8 contract is "drop into `/workspace/.intake/<intake_id>/` via the
  worker's first session's SandboxClient (resolved post-hoc; if no
  session yet, defer to attachment fetch on first task action)" — Phase
  2 has neither a sandbox at intake time (Initializer spawn is DEBT
  #13) nor a "fetch on first action" plumbing surface, so the simplest
  honest behaviour is to drop the bytes and surface a metadata
  manifest. Operators replying with attachments (e.g., "summarise this
  PDF") will get a deliverable that can't reference the original file
  content yet.
- **Why**: A staging dir under `data/intake/<intake_id>/` would work
  but introduces a new lifecycle question (TTL, GC, cross-session
  hand-off). Wiring through the SandboxClient lazily would be the
  right fix, but that requires either a Phase 2 DEBT #13 close-out (so
  there's a session to hand off to) or a brand-new "intake staging"
  store that Phase 2 doesn't otherwise need. Surfacing the attachment
  metadata while skipping bytes is the smallest honest stub.
- **Pay down**: Story 2.15 (provenance manifest builder) is the natural
  caller — when the manifest's evidence-event collection runs, it can
  also rehydrate intake attachments from a staging dir into the
  sandbox. Alternatively, story 2.8 (briefing confirm + Initializer
  spawn) closes DEBT #13, after which the IntakeRouter has a
  SandboxClient handle and EmailChannel can write directly into the
  per-session workspace. Whichever lands first owns the migration.

### ~~17. `AppState::with_channels` replaces (not merges) the chat baseline~~ — CLOSED in story 2.10
- **Origin**: story 2.9, `crates/seasoned-hand-server/src/lib.rs`
- **Severity**: **Low**
- **What**: `AppState::new` registers the always-on
  [`ChatChannel`](../../crates/seasoned-hand-core/src/channel/chat.rs)
  baseline into the [`ChannelRegistry`](../../crates/seasoned-hand-core/src/channel/mod.rs).
  The existing `with_channels(channels: ChannelRegistry)` builder method
  *replaces* `self.channels` wholesale, so any future caller that passes
  in a freshly-built registry containing webhook / email / cli / ntfy
  registrations will silently drop the chat entry.
- **Why**: No production caller invokes `with_channels` today — the
  only consumers of multi-channel registration land in stories
  2.10–2.13. Designing the merge surface now (and writing the tests
  for it) would be speculative.
- **Pay down**: Story 2.10 (`WebhookChannel`) is the first caller that
  needs additional channels at boot. That story should either:
  - replace `with_channels(registry)` with `with_channel(name, reg)` /
    `register_channel(...)` taking one registration at a time, OR
  - keep the `with_channels` shape but have it merge each entry on top
    of the existing registry rather than swap the Arc.
- **Resolution (story 2.10)**: Chose **option (a)** — `with_channels`
  is replaced by `AppState::register_channel(ChannelRegistration)`,
  which folds the new registration into a fresh registry built from
  the existing entries, then re-points `intake_router` /
  `delivery_router` at the merged Arc. The chat baseline registered
  by `AppState::new` (story 2.9) now survives every subsequent
  registration; `register_webhook_channel` is a convenience builder
  that constructs `WebhookChannel`, snapshots its intake token onto
  AppState, and calls `register_channel`. The `tests/channels.rs`
  fixture was migrated to the per-channel API and its assertions
  account for the chat baseline. No production caller invoked the
  old `with_channels` signature, so no compatibility shim was needed.

### 19. Task state machine widened: Drafted/Briefed/Confirmed → Cancelled
- **Origin**: story 2.8, `crates/seasoned-hand-core/src/project/task.rs`
  (`legal_transitions`)
- **Severity**: **n/a** (informational — spec-gap close-out, not a stub)
- **What**: The Phase 2 task lifecycle docstring claims
  `drafted → briefed → confirmed → running ⇄ paused → completed |
  failed | cancelled`, which on first reading suggests cancellation
  only from Running/Paused. Story 2.8's `BriefingAction::Cancel`
  requires cancelling **from `Briefed`** (the gate state), which the
  original `legal_transitions` matrix forbade — the
  `briefing_cancel_transitions_to_cancelled` unit test surfaced this
  with `IllegalTransition { from: Briefed, to: Cancelled }`.
- **Why**: The original matrix was too tight — a user cancelling
  during the briefing gate is a first-class flow per architecture
  §2.2 ("On `cancel`: task status → `cancelled`"). Widening to
  `Drafted/Briefed/Confirmed → Cancelled` matches the spec's
  cancel-anytime-before-running semantics.
- **Resolution (story 2.8)**: `legal_transitions` now lists
  `Cancelled` as a valid target from `Drafted`, `Briefed`, and
  `Confirmed` in addition to the existing `Running` / `Paused`. The
  pinned-table test (`task_state_machine_legal_transitions`) was
  updated to assert the new shape. No new column or migration — this
  is a code-level state machine refinement.

---

## Categories quick-reference (same as Phase 0 / Phase 1)

| Severity | Meaning |
|---|---|
| **H** | Blocks the next phase's goals if not addressed |
| **M** | Will bite at scale or in a year, manageable today |
| **L** | Documentation / minor friction / one-line fix later |
