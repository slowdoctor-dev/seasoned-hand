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

### ~~13. IntakeRouter does not spawn the Initializer~~ — CLOSED in story 2.8b
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
- **Status (story 2.8)**: ~~Still open.~~ Story 2.8 landed
  `Initializer::run_with_confirmation` (Briefing emit + confirm gate +
  edit cap + auto-confirm) as a self-contained method with 6 unit
  tests but deferred the IntakeRouter wiring to story 2.8b.
- **Resolution (story 2.8b)**: A new
  `InitializerSpawner` trait lives in
  `crates/seasoned-hand-core/src/intake/spawner.rs`. The `IntakeRouter`
  holds an `OnceLock<Arc<dyn InitializerSpawner>>` attached by
  `attach_initializer_spawner(...)` and invoked from
  `handle_event(...)` immediately after the drafted task is persisted +
  linked. `HandleOutcome::Created` grew a `session_id: Option<String>`
  that carries the spawner's receipt back to the caller. The
  server-side concrete impl
  [`WsInitializerSpawner`](../../crates/seasoned-hand-server/src/initializer_spawner.rs)
  inserts the sessions row, registers the per-task
  `mpsc::Sender<UserResponse>` in `AppState::briefing_senders`, and
  fires a fire-and-forget tokio task that runs
  `Initializer::run_with_confirmation` followed (on
  `RunOutcome::Started`) by `AgentRunner::resume(...)`. Spawner errors
  are non-fatal — the intake row + drafted task survive so the
  operator can retry without losing the original brief.

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

### ~~15. WS `task_create` still spawns the runner directly (Phase 0/1 shim)~~ — CLOSED in story 2.8b
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
- **Resolution (story 2.8b)**: The WS `task_create` handler in
  `crates/seasoned-hand-server/src/ws.rs` no longer inserts the
  sessions row or spawns `AgentRunner::run` directly. It pre-allocates
  a `session_id` (passed through `IntakeEvent.metadata.session_id_hint`
  so the chat reply_target `session:<id>` resolves to the same id),
  pushes the intake event into the `IntakeRouter`, and Acks with the
  spawner-derived session_id once `HandleOutcome::Created` returns.
  `max_steps` and `cost_cap_cents` ride along the same metadata
  envelope so the spawner's eventual `AgentRunner::resume(req)` call
  preserves the original chat-client knobs. The
  `task_create_returns_session_id_and_starts_runner` test was kept
  (still pins "session row exists post-Ack") and a new
  `ws_task_create_emits_briefing_then_confirm_acks_started` test
  exercises the briefing-emit → `briefing_confirm` → sender-removed
  loop end-to-end. The new `BriefingConfirm` WS verb (`{cmd:
  "briefing_confirm", task_id, in_reply_to_call_id, action, edits?}`)
  routes through `forward_briefing_confirm(...)` which reads
  `AppState::briefing_senders` keyed by task_id; unknown tasks Ack
  with `error: "no_pending_briefing"`.

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

### 20. Initializer confirm gate uses loose `in_reply_to_call_id` match
- **Origin**: story 2.8b, `crates/seasoned-hand-core/src/agent/init/mod.rs`
  (`run_confirm_gate`)
- **Severity**: **Low**
- **What**: The confirm gate consumes any `UserResponse` that arrives
  on the per-task mpsc, regardless of whether
  `response.in_reply_to_call_id` matches the current
  `briefing_call_id`. If the operator submits `confirm` on a card whose
  briefing was already superseded by an `edit` (and a fresh
  `briefing_call_id` minted), the stale confirm still progresses the
  gate.
- **Why**: Story 2.8 chose the loose match to keep the 6-test surface
  focused on action semantics; story 2.8b inherited that surface and
  preserved it to avoid expanding the keystone wiring beyond DEBT
  #13 / #15. A bounded mpsc with depth 8 caps the worst-case backlog
  but doesn't enforce ordering.
- **Pay down**: A future story tightens `run_confirm_gate` to drop
  responses with `in_reply_to_call_id != current_call_id`, looping
  back into `wait_for_response` instead of consuming them. Probably
  bundled with story 2.23 (FE emits the cmd) so the FE + BE
  call-id discipline ships together.

### 21. Non-chat channels don't forward briefing events to the user
- **Origin**: story 2.8b — webhook + email intake paths flow through
  the same `IntakeRouter` + `WsInitializerSpawner` but the
  `Briefing` / `briefing_pending` Misc events only surface to the WS
  chat subscriber.
- **Severity**: **Medium** (functional gap for non-chat intake)
- **What**: The Initializer emits `briefing` + `briefing_pending` Misc
  events into the per-session events stream. The ChatChannel
  subscriber receives them via the existing WS subscribe mechanism
  (the session_id matches). Webhook intake currently has no
  back-channel for these events — the 202 Accepted response is the
  only signal the caller sees. Email intake doesn't reply with a
  briefing card either; the user can't confirm / edit until the
  5-minute auto-confirm fires.
- **Why**: Story 2.8b's scope is the wiring keystone (router →
  spawner → mpsc). Adding per-channel briefing-forwarding for
  webhook (POST to `reply_target.url` with a render of the brief +
  return URL for confirm) and email (compose a reply with a
  confirmation link) is its own pair of stories.
- **Pay down**: Either (a) Phase 2 stretch story to add
  briefing-forward hooks on each `DeliverySink` impl, or (b) accept
  the 5-minute auto-confirm as the Phase 2 contract for non-chat
  intake and defer the interactive flow to Phase 4/5 with
  multi-user. Architecture §2.2 +
  `specs/phase-2/stories/story-2.8.md` "Non-goals" section both
  signpost this as out-of-scope for 2.8 itself.

### 22. e2e + phase1_gaia tests don't send `briefing_confirm`
- **Origin**: story 2.8b — pre-existing `#[ignore]` tests
  (`crates/seasoned-hand-server/tests/e2e_phase0.rs`,
  `tests/phase1_gaia.rs`) assume `task_create` auto-starts the runner.
- **Severity**: **Low** (CI-green; only affects opt-in live runs)
- **What**: Both tests send `task_create` and then wait for `Action` /
  Message events. Under the post-2.8b flow the runner only starts
  after a `briefing_confirm` arrives (or the 5-minute auto-confirm
  fires). Both tests have shorter deadlines (180 s + per-step
  timeouts), so a live run would hang on the briefing gate until
  auto-confirm.
- **Why**: The tests are gated `#[ignore]` and only run with explicit
  env opt-in (`SH_E2E_WS_URL`, `SEASONED_HAND_PHASE1_SMOKE=1`). The
  default `cargo test --workspace` skips them, so CI stays green.
  Fixing them properly means reading the `briefing_pending` Misc
  event and replying with a `briefing_confirm` cmd — a one-block
  diff per test, but not in this story's keystone scope.
- **Pay down**: Next time either test is run live, the operator (or a
  Phase 2 closeout story) sends a `briefing_confirm` cmd between
  receiving the first Misc event and waiting on Action events. The
  test can also opt into `RunConfig { require_confirm: false,
  confirm_timeout: Duration::from_millis(100) }` via a test-only env
  override if the manual confirm is undesirable.

### 24. Provenance `brief.confirmed`/`confirmed_at`/`edits_applied` are static placeholders
- **Origin**: story 2.15,
  `crates/seasoned-hand-core/src/provenance/builder.rs` (`build_manifest`)
- **Severity**: **Low**
- **What**: The manifest's `brief` block currently writes
  `confirmed = true`, `confirmed_at = None`, `edits_applied = 0` for
  every deliverable. The real confirm/edit lineage lives in the
  Initializer's `run_confirm_gate` (story 2.8 / 2.8b) but never makes
  it into a queryable surface that the manifest builder can read.
- **Why**: Tracing the gate's actual outcome would require either
  persisting confirm/edit/cancel cycles into a new column on `tasks`
  (Brief.confirmed_at, edits_applied) or replaying the `briefing` Misc
  event stream to count edits. Neither lands cleanly inside story 2.15
  without growing scope.
- **Pay down**: Story 2.23 (FE briefing card) or a Phase 3 close-out
  threads the gate's counters onto `tasks` (or a tiny side table) and
  the builder reads them. The schema slot is reserved in §2.11 already.

### 25. Provenance `intake` synthesizes "unknown" entry for legacy WS tasks
- **Origin**: story 2.15,
  `crates/seasoned-hand-core/src/provenance/builder.rs` (`load_intake`)
- **Severity**: **Low**
- **What**: When a task has no `intake_events` row (i.e. it was created
  via the legacy WS `task_create` shim before DEBT #15 closed; or via
  a future code path that bypasses intake) the builder emits a
  synthetic `IntakeProvenance { channel: "unknown", intake_id:
  "synthetic", received_at: task.created_at, metadata: {} }` rather
  than failing. Keeps the §2.11 schema rigid (`intake` always present)
  at the cost of losing the actual chat-side message-id evidence.
- **Why**: Phase 2 closed DEBT #15 (WS task_create now creates an
  intake_events row via ChatChannel) but pre-2.8b tasks + any future
  out-of-band creation path would otherwise fail manifest build.
- **Pay down**: Phase 5 multi-user — enforce intake row creation at
  every task-creation site, drop the synthetic fallback + return
  `ProvenanceError::IntakeMissing` on the cold path.

### 27. `task::resume_task` uses in-memory handle as "container exists" proxy
- **Origin**: story 2.16, `crates/seasoned-hand-core/src/task/resume.rs`
  (`SandboxOps::get_handle` branch)
- **Severity**: **Low**
- **What**: The rebuild-vs-unpause split looks at the in-memory
  `SandboxClient::handles` map. After a server process restart that
  map is empty even though the docker container may still be alive
  and only paused — `task_resume` will trigger a full sandbox rebuild
  + event-stream replay instead of cheaply unpausing the existing
  container.
- **Why**: Story 2.6 + Phase 2 §8 model "container alive" as "handle
  present"; rehydrating the map from `docker ps` (a la
  `SandboxClient::rehydrate_from_docker`, story 1.2) is the correct
  fix but adds boot-time docker round-trips out of scope for 2.16.
- **Pay down**: Phase 5 boot-time reconciliation — call
  `rehydrate_from_docker` BEFORE accepting WS connections so paused
  containers re-register, and the unpause path fires on cross-restart
  resume.

### 28. Replay cost baseline resets to zero on rebuild
- **Origin**: story 2.16, `crates/seasoned-hand-core/src/task/replay.rs`
  (`replay_cost_baseline`)
- **Severity**: **Low**
- **What**: The rebuild path starts the new session's `cost_cents` at
  0 instead of copying the old session's accumulated cost. A 24h task
  that's already burned $5 and rebuilds at hour 23 effectively gets
  a fresh budget for the final hour — cost-cap accounting under-counts
  cumulative spend.
- **Why**: Phase 0/1 `CostClient` has no per-session baseline state
  (the baseline is a per-loop `CostSnapshot` value). Persisting cost
  snapshots as Misc events + replaying them needs a new emit site that
  story 2.16 is explicitly scoped not to add. The user-prompt for 2.16
  noted this trade-off and asked for the zero-reset path.
- **Pay down**: Phase 3 — add a periodic `cost_snapshot` Misc emitted
  by the runner loop; `replay_cost_baseline` reads the latest snapshot
  for the old session and writes the matching delta onto the new
  session's `cost_cents`. Until then, cost caps reset per rebuild.

### 23. CliChannel not registered into the production AppState
- **Origin**: story 2.13, `crates/seasoned-hand-core/src/channel/cli.rs`
- **Severity**: **Low**
- **What**: `CliChannel` is built + unit-tested but never registered
  into `AppState.channels` — the `register_cli_channel` builder
  doesn't exist yet. Without registration, the channel framework's
  `DeliveryRouter` can't route a `cli` reply_target, so the only
  consumer is in-process tests.
- **Why**: The CLI binary itself lands in story 2.21
  (`seasoned-hand-cli` crate). Until that binary exists nothing
  calls `CliChannel::register_pending(...)` + pushes an IntakeEvent,
  so registering the channel now would leave dead routing slots
  visible at `GET /v1/channels`. Story 2.13's spec ACs treat
  registration as "when CLI subcommand launches the server" —
  meaning the future binary's main(), not the current
  headless-server main.rs.
- **Pay down**: Story 2.21 adds `AppState::register_cli_channel()` +
  calls it from the CLI binary's `task new` path. The same
  `Arc<CliChannel>` is shared between the in-process IntakeEvent
  push site and the registered `DeliverySink` slot.

---

## Categories quick-reference (same as Phase 0 / Phase 1)

| Severity | Meaning |
|---|---|
| **H** | Blocks the next phase's goals if not addressed |
| **M** | Will bite at scale or in a year, manageable today |
| **L** | Documentation / minor friction / one-line fix later |
