# Phase 2 — Technical Debt Ledger

> Append-only list of shortcuts, stubs, simplifications, and deferred
> work introduced during Phase 2. Same discipline as Phase 0 / Phase 1
> DEBT.md.
>
> Seeded at architecture phase boundary (2026-05-13). Items added during
> story implementation get appended below the seed block.

## Closeout audit (story 2.27, 2026-05-16)

Of 22 in-phase entries appended during Phase 2 (numbered #10–#32
with #26 skipped), eight graduated to **closed** during the phase
itself; each strike-through now links its closing story SHA:

- **#10** (`Deliverable` struct location) → story 2.3 (`2c36eae`)
- **#11** (`mark_delivered` row-existence stub) → story 2.5 (`030ffcd`)
- **#13** (IntakeRouter doesn't spawn Initializer) → story 2.8b (`d1006ff`)
- **#15** (WS `task_create` legacy shim) → story 2.8b (`d1006ff`)
- **#16** (IntakeRouter `run` loop not spawned) → story 2.10 (`b19ec7f`)
- **#17** (`with_channels` replaces chat baseline) → story 2.10 (`b19ec7f`)
- **#23** (CliChannel not registered) → story 2.21a (`527ff75`)
- **#32** (`rendered_content_path` workspace-relative vs absolute) →
  story 2.26 (`27d3770`)

**#12** sits partially closed (4xx surface lands in story 2.10; Misc
emit deferred until a system-session anchor exists). **#19** is
informational only (task-state widening — a code-level state-machine
refinement that closed a spec gap, not a stub or shortcut). **#26** is
a skipped number (the EmailChannel #18 → state-machine #19 →
Initializer #20 → ... renumbering went straight from 25 to 27; no
entry exists). **All other in-phase entries** (#14, #18, #20, #21,
#22, #24, #25, #27, #28, #29, #30, #31) remain **open** and are
scheduled to specific later phases on each entry's `Pay down` line —
these survived a full Phase 2 retrospective review and are honestly
open as of 2026-05-16. #18 (EmailChannel attachment bytes) is Medium
severity and the most likely Phase 3 pay-down candidate of the
in-phase open set.

Seed items #1–#9 also remain **open** at Phase 2 close. Each is
deliberately routed to Phase 3, 4, or 5 per its `Pay down` line:
**#1, #4, #8** Phase 5 multi-user; **#2, #3** Phase 4 once toolchain
stabilises; **#5** Phase 3+; **#6** Phase 3 fills the table; **#7**
addressed below; **#9** informational. None were resolved in Phase 2
proper.

**Phase 1 DEBT #3** (verifier rollback default flip) carries to Phase
3 (recorded as item #7 above). Story 2.26 landed the
`phase2-live-overnight` workflow_dispatch jobs but the jobs are
operator-triggered and have not accumulated verdict precision data
yet. The default in `crates/seasoned-hand-server/src/lib.rs`
(`checkpoint_rollback_on_verifier_fail = false`) is **unchanged**;
Phase 3 retro should re-evaluate once enough live runs exist.

### Post-close hardening pass (2026-05-16)

`/specs/phase-2/REVIEW.md` (commit `dabd4cf`) surfaced **15 additional
items** (entries #33–#47 below). The hardening pass that followed
(`fc75ae4` + `0714dbf` + `cbb7b77`) closed **8** of them:

- **#33** FE timestamp unit drift → `fc75ae4`
- **#34** Provenance route loopback → `fc75ae4`
- **#35** Webhook `session_id_hint` path-traversal → `0714dbf` (**H**)
- **#36** `target_filename` shell-inject / bind-mount → `0714dbf` (**H**)
- **#37** Admin-token constant-time → `fc75ae4`
- **#38** `normalize_workspace_relative_path` `..` block → `0714dbf`
- **#41** `mark_delivered` rename + doc fix → `cbb7b77`
- **#42** `.env.example` Phase 2 vars → `cbb7b77`

**7 remain open** at the end of the hardening pass: **#39** (WS
session_id vs task_id reconciliation), **#40** (5 spec'd HTTP routes
missing), **#43** (WS `briefing_confirm` no auth — covered by Phase 0
DEBT #7 umbrella), **#44** (story 2.5 `RouteOutcome<T>` unmet), **#45**
(story 2.12 missing 2 live-Redis tests), **#46** (EmailChannel
absolute-path containment check), **#47** (DNS-rebinding TOCTOU). Each
carries an explicit pay-down line in its entry below. The two H-severity
items closed in the hardening pass moved Phase 2 from "Medium-trust
single-operator with two host-escape primitives" to "Medium-trust
single-operator with no known host-escape primitives".

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

### ~~10. `Deliverable` struct lives inside `channel/delivery.rs`~~ — CLOSED in story 2.3 (`2c36eae`)
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

### ~~11. `DeliverableStore::mark_delivered` is a row-existence stub~~ — CLOSED in story 2.5 (`030ffcd`)
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

### ~~13. IntakeRouter does not spawn the Initializer~~ — CLOSED in story 2.8b (`d1006ff`)
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

### ~~15. WS `task_create` still spawns the runner directly (Phase 0/1 shim)~~ — CLOSED in story 2.8b (`d1006ff`)
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

### ~~16. IntakeRouter `run` loop + shared mpsc not yet spawned in `main.rs`~~ — CLOSED in story 2.10 (`b19ec7f`)
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

### ~~17. `AppState::with_channels` replaces (not merges) the chat baseline~~ — CLOSED in story 2.10 (`b19ec7f`)
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

### ~~23. CliChannel not registered into the production AppState~~ — CLOSED in story 2.21a (527ff75)
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
- **Resolution (story 2.21a)**: `AppState` now owns
  `cli_channel: Arc<CliChannel>` (built by `AppState::new`).
  `AppState::register_cli_channel(self) -> Self` registers the same
  Arc into the `ChannelRegistry` under both intake and delivery
  slots; `main.rs` calls it once after `register_ntfy_channel` so
  the headless production server always exposes the `cli` slot via
  `GET /v1/channels`. Story 2.21b's `task new --blocking` will read
  `AppState::cli_channel.register_pending(...)` directly to set up
  the in-process oneshot path.

### 29. `task new --no-auto-confirm` metadata flag not honored by spawner
- **Origin**: story 2.21b,
  `crates/seasoned-hand-cli/src/commands/task.rs` (the `--no-auto-confirm`
  flag) +
  `crates/seasoned-hand-server/src/initializer_spawner.rs` (uses
  `RunConfig::default()` unconditionally)
- **Severity**: **Low**
- **What**: The CLI accepts `seasoned-hand task new "<brief>"
  --no-auto-confirm` and records `metadata.no_auto_confirm = true` on
  the IntakeEvent, but `WsInitializerSpawner::spawn` builds
  `RunConfig::default()` (which has `require_confirm: false`) without
  looking at `spec.metadata`. The 5-minute auto-confirm timer still
  fires regardless of the flag.
- **Why**: Threading metadata into `RunConfig` would touch
  `SpawnSpec` + the spawner + the per-call shape of `RunConfig` —
  out-of-scope for 2.21b's "CLI surface" charter. The flag is wired
  end-to-end in the wire format so operators can rely on it landing
  later without a UX change.
- **Pay down**: Add `SpawnSpec::require_confirm: bool` (default
  `false`); `IntakeRouter::handle_event` reads
  `event.metadata.no_auto_confirm` and flips the bool;
  `WsInitializerSpawner::spawn` builds `RunConfig { require_confirm:
  spec.require_confirm, .. }`. One-story scope; bundle with a future
  story that's already touching the spawner.

### 31. BriefingCard has three rough edges (eviction / reload / server-error UX)
- **Origin**: story 2.23,
  `frontend/components/chat/briefing-card.tsx` +
  `frontend/components/chat.tsx`
- **Severity**: **Low**
- **What**: Three small gaps left in the Phase-2 briefing UI:
  1. **Eviction**. `useAgentSocket` caps the per-tab event buffer at
     `EVENTS_CAP = 1000` (FIFO). If a session emits 1000+ events
     before a Briefing renders, the older `briefing_pending` sidecar
     may be evicted; the card then can't resolve `task_id` and the
     three buttons disable with "Cannot resolve briefing — task id
     not yet known". Phase 2 doesn't hit this in practice (the
     briefing fires within the first 5-10 events of a task) but the
     failure mode is silent.
  2. **Reload**. The Confirm/Cancel optimistic resolution lives in
     React state (`localResolutions`). On reload it's lost: the card
     reverts to "Pending" and the three buttons re-enable, even
     though the server has long since moved on. Clicking Confirm
     again returns `no_pending_briefing` and the card finally settles
     on the error row.
  3. **Server-side validation errors**. If the Edit JSON parses but
     fails `Brief::validate` server-side (e.g. `goal_empty`, the
     `briefing_invalid{reason:"…"}` Misc), the card never sees it —
     the response lands as generic Misc text in the chat scroller
     (decision/task_state-style snippet) below the still-active card.
- **Why**: All three need plumbing the FE doesn't have yet. (1)
  requires either a backfill HTTP fetch on missing call_id or
  promoting `task_id` to the Briefing event itself (server-side
  spec change). (2) requires persisting resolution to localStorage
  or rebuilding it from a server-emitted Misc on confirm/cancel
  (today neither verb echoes back a Misc the FE can listen for —
  only the downstream `task_state` flips). (3) requires correlating
  `briefing_invalid` Misc events with their preceding Edit cmd's
  `briefing_call_id` (the server emits these without a back-pointer
  today).
- **Pay down**: Pick the highest-impact one first. (3) is cheapest:
  add `briefing_call_id` to the `briefing_invalid` Misc payload in
  `Initializer::run_confirm_gate` so the card can intercept and
  show inline. (2) follows by either persisting confirmed/cancelled
  call_ids to localStorage or having the server emit
  `briefing_confirmed`/`briefing_cancelled` Misc events alongside
  the `task_state` transition (cleaner — server is single source of
  truth). (1) is the smallest practical risk; defer until a session
  actually trips it.

### 30. `seasoned-hand channel logs` is a stub
- **Origin**: story 2.21b,
  `crates/seasoned-hand-cli/src/commands/channel.rs`
- **Severity**: **Low**
- **What**: `seasoned-hand channel logs <NAME> [--tail]` exits with an
  explanatory error (`channel logs is not yet implemented — see
  phase-2/DEBT.md #30`). The other `channel` subcommands (`list`,
  `test`) are live; only `logs` is deferred.
- **Why**: The server has no per-channel structured log feed today
  (channels log via `tracing` directly to the process's
  stdout/stderr). Adding a WS-subscription endpoint + the matching
  CLI subscriber overran 2.21b's 2-3h budget; the rest of the CLI
  surface is more load-bearing.
- **Pay down**: Add `GET /v1/channels/:name/logs` as a WS upgrade that
  drips a filtered structured-log stream (channel name + role +
  rendered tracing fields). CLI side: replace the stub with a
  `tokio-tungstenite` subscriber that prints `--tail`-style output.
  Could also drop the `tail` flag and have the route default to live
  tail if the cost is the same.

### ~~32. `task_deliver` stores workspace-relative `rendered_content_path` but `EmailChannel::deliver` reads it as absolute~~ — CLOSED in story 2.26 (`27d3770`)
- **Origin**: surfaced by story 2.25 (`crates/seasoned-hand-server/tests/phase2_overnight.rs`),
  rooted in `crates/seasoned-hand-core/src/deliverable/task_deliver.rs`
  (writes `rendered.workspace_path` verbatim) +
  `crates/seasoned-hand-core/src/channel/email/mod.rs::deliver`
  (calls `tokio::fs::read(&deliverable.rendered_content_path)` directly).
- **Severity**: **Medium** (only bites real email-channel deliveries; chat /
  webhook handlers don't read file bytes, so default `cargo test` is clean
  outside this overnight test).
- **What**: `task_deliver` persists `rendered_content_path` as the
  workspace-relative form returned by `fingerprint_artifact` (e.g.
  `.deliverables/phase2-summary.docx`). The Phase 0/1 frontend / chat
  consumers only round-trip that string as a `file_ref` field (no I/O), so
  this was never noticed. `EmailChannel::DeliverySink` is the first
  consumer that reads the bytes off disk — and it treats the column value
  as an absolute path. Live email delivery fails with
  `No such file or directory` for every overnight workflow that doesn't
  happen to be CWD-rooted at the workspace.
- **Why**: Architecture §2.7 / §2.9 don't pin whether `rendered_content_path`
  is workspace-relative or absolute. Chat / webhook channels stayed
  forgiving by accident. Fixing this inside 2.25 would either widen
  `task_deliver`'s contract (resolve via `SandboxClient::workspace_host_path`
  before persisting) or thread a `SandboxClient` reference into
  `EmailChannel::deliver` — either change touches surfaces beyond the
  acceptance-gate test's scope.
- **Resolution (story 2.26)**: `task_deliver` now resolves the
  workspace-relative path returned by `RendererDispatcher` against
  the sandbox handle's `workspace_host_path` immediately before
  constructing `NewDeliverable`. The persisted column is the absolute
  on-disk path, so `EmailChannel::deliver` (and any future consumer
  that performs I/O on the rendered bytes) succeeds with
  `tokio::fs::read(...)`. The `canonicalize_rendered_path` UPDATE
  workaround in `tests/phase2_overnight.rs` is removed; a unit-level
  regression test asserts `row.rendered_content_path.is_absolute()`
  in
  `crates/seasoned-hand-core/src/deliverable/task_deliver.rs::tests::task_deliver_writes_source_and_renders`.

---

## Post-close review (REVIEW.md, 2026-05-16)

Seeded by `/specs/phase-2/REVIEW.md` (commit `dabd4cf`) — an external
4-dimension audit of the Phase 2 close. Entries #33–#47 below were
identified but not silently appended during the audit; this section
records each with a current status. Entries closed by the hardening
pass (commits `fc75ae4`, `0714dbf`, `cbb7b77`) carry strikethrough +
the closing SHA.

### ~~33. Frontend timestamp unit drift (μs ↔ s ↔ ms)~~ — CLOSED in hardening pass 2 (`fc75ae4`)
- **Origin**: REVIEW §3 cross-finding A
- **Severity**: **Medium** (silent user-facing display bug)
- **What**: Backend persists all timestamps in microseconds since
  unix epoch (events, deliverables, tasks, intake / delivery / notify
  events, verifications, WS `ServerEvent.ts`). Two FE consumers
  treated them as other units: `decisions-tab.tsx:90` multiplied μs
  by 1000 (→ ns, far-future dates) and `verifier-tab.tsx:228` treated
  μs as seconds. The latter is a Phase 1 carry-over surfaced by 2.22.
  Separately, WS `ServerPing/Pong.ts` is unix seconds — same JSON
  field name, different unit depending on `type` discriminator.
- **Resolution (hardening pass 2)**: Both FE sites flipped to `/ 1000`
  (μs → ms); `verifier-tab.spec.ts` fixture flipped from 1.7e9
  (seconds) to 1.7e15 (microseconds). The `ServerPing/Pong.ts` unit
  drift remains documented but not changed — promoting it to μs would
  break wire format for zero readability gain (FE never displays
  ping timestamps).

### ~~34. `GET /v1/tasks/:id/provenance` not loopback-gated~~ — CLOSED in hardening pass 2 (`fc75ae4`)
- **Origin**: REVIEW §1/C
- **Severity**: **Medium**
- **What**: Every sibling `/v1/tasks/:id/*` route applied
  `require_loopback(remote)?`; `get_task_provenance_handler` alone
  did not. Provenance manifests can include PII (sender addresses,
  brief content, intake metadata).
- **Resolution (hardening pass 2)**: Added the loopback guard with
  the same `ConnectInfo` extractor + comment cross-referencing the
  REVIEW finding.

### ~~35. Webhook intake `session_id_hint` path-traversal~~ — CLOSED in hardening pass 3 (`0714dbf`)
- **Origin**: REVIEW §1/G
- **Severity**: **High** (only bounded by the webhook intake token
  + Phase 2 single-operator)
- **What**: `IntakeEvent.metadata.session_id_hint` flowed verbatim
  into `sessions.id` (PK), then into `workspace_root.join(session_id)` →
  `tokio::fs::remove_dir_all(...)` on the TTL cron, plus the docker
  container name `seasoned-hand-sandbox-<id>`. An attacker holding
  the webhook intake token (or any future Phase 5 untrusted caller)
  could plant `"../../../tmp/poisoned"` and trigger host-side
  `rm -rf` outside the workspace bind-mount.
- **Resolution (hardening pass 3)**: `is_safe_session_id(...)` in
  `intake/router.rs` accepts only `[A-Za-z0-9-]+` of length 1..=64
  before the value reaches `SpawnSpec`. Drop-not-error so the intake
  still creates the task (spawner mints a fresh UUID). New regression
  test `intake_router_drops_unsafe_session_id_hint` exercises 1 safe
  + 6 bad shapes.

### ~~36. LLM `target_filename` shell-injection / bind-mount escape~~ — CLOSED in hardening pass 3 (`0714dbf`)
- **Origin**: REVIEW §1/B
- **Severity**: **High** (only bounded by the LLM trust boundary
  + Phase 2 single-operator)
- **What**: `task_deliver` only extension-validated the filename. The
  raw string flowed into Pandoc / python-pptx / openpyxl shell
  commands and into `workspace_host_path.join(...)`. Shell
  metacharacters like `;`, `$(...)`, backticks, pipes would execute
  inside the sandbox; `..` segments would escape the workspace
  bind-mount on the host.
- **Resolution (hardening pass 3)**: `validate_deliverable_filename(...)`
  in `deliverable/task_deliver.rs` enforces `[A-Za-z0-9._-]+`, length
  ≤ 120, no leading dot, no `..`, must contain an extension. Surfaces
  via `invalid_filename:<reason>` tool-error sub-code. New regression
  test covers 10+ accept/reject cases.

### ~~37. Admin-token compare uses `!=` not constant-time~~ — CLOSED in hardening pass 2 (`fc75ae4`)
- **Origin**: REVIEW §1/C
- **Severity**: **Medium** (mitigated by loopback guard)
- **What**: Admin rollback + sandbox cleanup handlers used
  `token_hdr != Some(state.admin_token.as_str())` — prefix-length
  timing leak. Webhook intake already used `subtle::ConstantTimeEq`;
  inconsistent.
- **Resolution (hardening pass 2)**: Both admin sites now use
  `subtle::ConstantTimeEq::ct_eq` with empty-string fallback for
  missing header. 11 admin-token tests still green.

### ~~38. `normalize_workspace_relative_path` does not strip `..`~~ — CLOSED in hardening pass 3 (`0714dbf`)
- **Origin**: REVIEW §1/G
- **Severity**: **Medium**
- **What**: The helper at `sandbox/mod.rs:585` stripped only
  `/workspace/` and leading `/` — `..` segments passed through
  unchanged. Callers (`read_workspace_file`, `write_workspace_file`)
  joined the result to `workspace_host_path`, enabling traversal
  outside the bind-mount if filename validators were ever bypassed.
- **Resolution (hardening pass 3)**: Signature changed to
  `Result<&str, SandboxError>` rejecting any `Component::ParentDir`
  and any null byte. Two callers propagate. New regression test in
  `sandbox/tests.rs`. Defense-in-depth — DEBT #36 closes the
  primary `target_filename` path; this closes the residual sandbox
  helper path.

### 39. WS `task_pause/resume/cancel` use `session_id`, spec says `task_id`
- **Origin**: REVIEW §3 cross-finding 4
- **Severity**: **Medium** (functional gap, not security)
- **What**: Architecture §4 lines 853–855 specify `{cmd:"task_pause",
  task_id, durable?}` etc. Impl at `ws.rs:65-75` uses `session_id`
  (Phase 1 carry-over from story 1.17). HTTP siblings at
  `lib.rs:1294-1302` correctly use `task_id`. Functionally identical
  for single-session tasks; diverges for multi-session pause-resume
  cycles.
- **Pay down**: Either reconcile the WS shape (breaking change for
  any current clients) or update architecture §4 to document the
  divergence. Phase 3 warm-up scope.

### 40. Five spec'd HTTP routes missing or substituted
- **Origin**: REVIEW §3 cross-finding 6
- **Severity**: **Medium**
- **What**: Architecture §4 promises `GET /v1/projects/:id` (missing),
  `PATCH /v1/projects/:id` (substituted by one-way
  `POST /v1/projects/:id/archive` — no rename / un-archive flow),
  `GET /v1/tasks/:id/notifications` (missing),
  `GET /v1/tasks/:id/intake` (missing),
  `GET /v1/tasks/:id/deliveries` (missing),
  `GET /v1/notify/config` (missing). The FE doesn't currently need
  them, but the externally-visible API surface is silently smaller
  than the spec.
- **Pay down**: Phase 3 warm-up — either ship the routes (each is
  ≤ 30 lines of handler + store reads) or update architecture §4 to
  shrink to the actual surface.

### ~~41. `mark_delivered` lies + `rendered_content_path` doc lies~~ — CLOSED in hardening pass 4 (`cbb7b77`)
- **Origin**: REVIEW §4
- **Severity**: **Low**
- **What**: `DeliverableStore::mark_delivered` was renamed-needed
  since DEBT #11 closed in story 2.5 — the body is a `SELECT 1`
  existence probe, not a mark. Separately, `Deliverable.rendered_content_path`
  doc claimed "sandbox-relative" while story 2.26 / DEBT #32 made the
  field absolute.
- **Resolution (hardening pass 4)**: Renamed to `assert_exists` (one
  caller in `delivery/router.rs`, one test rename). Doc-comment on
  the field now describes the absolute-path semantics + cross-refs
  the canonicalize-on-persist site.

### ~~42. `.env.example` missing every Phase 2 env var~~ — CLOSED in hardening pass 4 (`cbb7b77`)
- **Origin**: REVIEW §2 "Env-var knob audit"
- **Severity**: **Low**
- **What**: 18+ vars read in production (`SEASONED_HAND_INTAKE_TOKEN`,
  IMAP_*, SMTP_*, EMAIL_*, INTAKE_EMAIL_ALLOWED_SENDERS,
  WEBHOOK_DELIVERY_ALLOWLIST, NTFY_*, SANDBOX_TTL_*,
  SANDBOX_CLEANUP_INTERVAL_SEC, SANDBOX_SKIP_RENDERER_INSTALL,
  CLI_INTAKE_MAX_WAIT_SECS, VERIFIER_MAX_CONCURRENCY,
  NARRATOR_PROMPT_PATH, SEASONED_HAND_ADMIN_TOKEN, …) but none in
  `.env.example`. Operators had to read architecture.md §9.
- **Resolution (hardening pass 4)**: Added Phase 2 sections to
  `.env.example` with all vars + sane defaults + 1-line purpose +
  DEBT references.

### 43. WS `briefing_confirm` has no auth (Phase 0 DEBT #7 widening)
- **Origin**: REVIEW §1/H
- **Severity**: **Medium**
- **What**: Phase 0 DEBT #7 (no WS auth) is the umbrella. Phase 2
  added `{cmd:"briefing_confirm", task_id, action, edits?}` which
  routes by attacker-controlled `task_id`. At `HOST=0.0.0.0` bind,
  anyone reaching `/ws` can confirm/edit/cancel any in-flight
  briefing by id. The HTTP sibling at `/v1/briefings/:id/confirm` IS
  loopback-gated — inconsistent. task_id is a UUID so practical
  exploit needs discovery.
- **Pay down**: Phase 5 multi-user WS auth (DEBT #7) closes the
  umbrella. Until then, operators on non-loopback binds should
  firewall the WS port.

### 44. Story 2.5 `RouteOutcome<T>` requirement unmet on channels routes
- **Origin**: REVIEW §3 stories 2.1-2.13 cross-finding 1
- **Severity**: **Low**
- **What**: Story 2.5 spec explicitly required the channels HTTP
  routes (`GET /v1/channels`, `GET /v1/channels/:name/health`,
  `POST /v1/channels/:name/test`) to use the Phase 1 simplicity-pass
  `RouteOutcome<T>` wrapper. Impl uses plain
  `Json<...>` / `(StatusCode, Json<ApiError>)` tuples.
- **Pay down**: One-story migration to `RouteOutcome` for the 3
  channel handlers; bundle with the missing routes from DEBT #40.

### 45. Story 2.12 missing 2 `#[ignore]` live-Redis worker tests
- **Origin**: REVIEW §3 stories 2.1-2.13 cross-finding 8
- **Severity**: **Low**
- **What**: Story 2.12 spec listed `notify_worker_consumes_and_dispatches`
  and `notify_worker_xacks_on_dispatch_error` as `#[ignore]` live-Redis
  tests. Neither exists. The XREADGROUP loop is covered indirectly
  via in-memory `handle_request_*` tests.
- **Pay down**: Add the two tests when a live-Redis CI surface lands
  (Phase 3 may exercise this anyway via learning-pipeline integration
  tests).

### 46. EmailChannel reads absolute `rendered_content_path` without workspace-containment check
- **Origin**: REVIEW §1/D
- **Severity**: **Medium**
- **What**: After DEBT #32 close, `task_deliver` resolves to absolute
  via `handle.workspace_host_path.join(...)`. `EmailChannel::deliver`
  trusts the DB row without re-asserting that it canonicalizes under
  the workspace root. A DB tamper (or a future `task_deliver` change
  that forgets the resolve) could read `/etc/passwd` and email it.
  DEBT #36 + #38 close the path-injection vectors that would write
  such a row today; this is the residual read-side check.
- **Pay down**: Phase 5 multi-user must canonicalize + assert the
  path lives under the per-session workspace root before
  `tokio::fs::read`.

### 47. DNS-rebinding TOCTOU between SSRF guard and reqwest send
- **Origin**: REVIEW §1/A
- **Severity**: **Low**
- **What**: `ssrf::assert_public_address(...)` resolves the URL's
  host and iterates the result. `self.http.post(url).send()` then
  re-resolves at send time via reqwest's own DNS path. An attacker
  controlling DNS for the host can return a public IP to the guard
  and a private IP to reqwest. Single-operator + operator-supplied
  URLs → effectively safe today.
- **Pay down**: Phase 5 hardening — resolve once and pin via a
  custom reqwest resolver that uses the verified IP.

---

## Pre-Phase-3 cross-phase review (REVIEW.md, 2026-05-16)

Seeded by `/specs/REVIEW.md` (the cross-phase pre-Phase-3 audit that
followed the Phase 2-specific REVIEW.md). Entries #48–#64 below are
the new findings from that pass. The hardening commits that closed
items in this pass are referenced inline:

- `18d472d` — fix(security): loopback-gate workspace + sessions
- `4b6f932` — docs(readme): flip phase status to Phase 2 → Phase 3
- `f99574e` — docs(glossary): add 7 Phase 2 terms
- `5e1d790` — docs(plan): module preamble
- `becf4da` — docs: WHY-comments on Phase 0/1 constants
- `082688c` — style(tools): drop WHAT-only section dividers
- `66cdc3f` — docs(agent): /// summaries on Phase 0/1 public types

Eight of seventeen items closed in this pass; the rest remain open
and route to either Phase 3 polish, Phase 5 multi-user, or
human-approval gates (`AGENTS.md` / `ARCHITECTURE.md` are on the §9
NEVER list).

### ~~48. `/v1/workspace/:session_id/*` not loopback-gated~~ — CLOSED in cross-phase hardening pass (`18d472d`)
- **Origin**: REVIEW §1 Section A
- **Severity**: **Medium** (only bounded by Phase 2 single-operator +
  default `HOST=127.0.0.1`)
- **What**: Every `/v1/tasks/:id/*` sibling used
  `require_loopback(remote)?`; the Phase 0-era workspace proxy did
  not. On `HOST=0.0.0.0` binds anyone could read deliverables,
  prompts, intermediate code via a guessable session UUID.
- **Resolution**: Added the `ConnectInfo<SocketAddr>` extractor +
  `require_loopback(remote)?` call to `workspace_root` /
  `workspace_proxy`. Regression test
  `workspace_root_refuses_non_loopback_remote` added.

### ~~49. AGENTS.md §13/§14 + README.md phase status stale~~ — CLOSED in cross-phase hardening pass (README half `4b6f932`; AGENTS.md half ADR-011 commit)
- **Origin**: REVIEW §3 Section H
- **Severity**: **Medium**
- **What**: After Phase 0/1/2 closed, README.md announced
  "Phase -1 — Planning complete. Phase 0 starting" and AGENTS.md §13
  said "Phase: -1 (planning) → Phase 0 starting"; AGENTS.md §14 listed
  "ADR-001 to ADR-008" though ADR-009 + ADR-010 exist.
- **Resolution**:
  - README.md half (`4b6f932`): phase status flipped to "Phase 2
    complete → Phase 3 starting", Quick-start unblocked, ADR list
    bumped to ADR-010.
  - AGENTS.md half (ADR-011 commit): §13 phase status flipped to
    "Phase 2 complete → Phase 3 starting", next-milestone updated to
    the Phase 3 architecture pass; §14 ADR list bumped to ADR-011 +
    added cross-ref to `/specs/REVIEW.md` and the v1.1 ARCHITECTURE
    note.

### ~~50. GLOSSARY missing 5-7 load-bearing Phase 2 terms~~ — CLOSED in cross-phase hardening pass (`f99574e`)
- **Origin**: REVIEW §3 Section G
- **Severity**: **Medium**
- **What**: `ChannelRegistration`, `IntakeRouter`, `DeliveryRouter`,
  `NotifyWorker`, `WorkspaceTtlCron`, `Provenance manifest`, and the
  `Brief` vs `Briefing` distinction were each referenced 5-15× in
  specs + code but absent from `/GLOSSARY.md`.
- **Resolution**: Seven new entries alphabetically (six architecture
  pieces + one core-concept split). Drive-by fix: corrected the
  Event-stream entry from "7 types total" to 8 with a note that
  `Knowledge`/`Datasource`/`Skill` are Phase 3+ Curator slots.

### ~~51. ARCHITECTURE.md v1.0 text drift — consolidated~~ — CLOSED in cross-phase hardening pass (ADR-011 commit)
- **Origin**: REVIEW §3 Section A
- **Severity**: **Medium**
- **What**: Three cross-phase items belong in one consolidated
  ADR-011 + v1.1 bump:
  - §2.2 sessions states list 5; V004 + code have 6 (`VERIFYING`
    added by Phase 1 story 1.9 via the table-recreate dance at
    `migrations/V004__verifications.sql:25-47`).
  - `TaskStatus` 8-variant Phase 2 state machine
    (`Drafted/Briefed/Confirmed/Running/Paused/Completed/Failed/Cancelled`,
    `crates/seasoned-hand-cli/src/format.rs:21-29`) is not described
    in the immutable doc — only `sessions.state` is.
  - Existing Phase 1 DEBT #1 (tool count 32→38) + #2 (Next.js 15→16)
    text drifts.
- **Resolution (ADR-011 commit)**: ADR-011 documents the v1.0 → v1.1
  text-drift consolidation. ARCHITECTURE.md amendments: §2.2 sessions
  states gain `VERIFYING` with V004 cross-ref; new §2.2.1 describes the
  Phase 2 `tasks` table + 8-variant `TaskStatus` state machine with
  cross-refs to V006 + the legal-transitions matrix; §2.4 + §7 tool
  count bumped to 38 with the Phase 1/2 breakdown enumerated.
  BASELINE.md §4 frontend stack flipped Next.js 15 → 16 (closes Phase 1
  DEBT #2). Phase 1 DEBT #1 (tool count drift) also closed via the §7
  enumeration. No code change — purely text reconciliation.

### 52. `crates/seasoned-hand-server/src/lib.rs` 2879-line split
- **Origin**: REVIEW §4 Section F
- **Severity**: **Medium**
- **What**: The Phase 0+1+2 HTTP surface (~40 routes + handlers +
  helpers) all lives in one file. Largest prod file in the repo. Diff
  review + merge-conflict cost will grow superlinearly as Phase 3
  adds learning-API handlers.
- **Pay down**: Phase 3 warm-up — split into
  `lib/{tasks,projects,channels,admin,workspace,intake,delivery}.rs`.
  ~8-12 hours; defers cleanly.

### ~~53. `plan/mod.rs` missing module doc-block~~ — CLOSED in cross-phase hardening pass (`5e1d790`)
- **Origin**: REVIEW §2 Module charters
- **Severity**: **Low**
- **What**: 27 of 28 modules in `seasoned-hand-core/src/` had a 3-20
  line `//!` preamble citing spec + ADRs + closing story SHA.
  `plan/mod.rs` started with `use std::sync::Arc;` — bare.
- **Resolution**: 13-line preamble citing ADR-010 + ARCHITECTURE.md
  §2.3 + story 1.1 + the Phase 0 DEBT #25 close.

### 54. `SimplifyLlm` trait collapse to concrete + `#[cfg(test)]` mock
- **Origin**: REVIEW §2 Trait surfaces
- **Severity**: **Low**
- **What**: 1 prod impl (`PlannerSimplifyLlm`) + 1 test impl
  (`RecordingSimplify`); the trait shape is pure test seam.
- **Status (this pass)**: **Deferred.** Closer inspection: the test
  impl carries real value (records prompt shape, returns canned
  content), and any collapse would either introduce an enum variant
  or restructure the test fixture. The Phase 2 REVIEW's "~60 LOC
  saved" estimate was optimistic — actual savings ~20 LOC. L-severity;
  not worth the test-surface churn before Phase 3.
- **Pay down**: Phase 3+ if/when the renderer-simplify path grows a
  second prod variant.

### 55. `ToolMaskPolicy` collapse or data-driven
- **Origin**: REVIEW §2 Trait surfaces
- **Severity**: **Low**
- **What**: 1 prod impl (`DefaultMaskPolicy`) + 0 test impls; the
  20-line match statement could be a `const MASK_RULES` slice.
- **Status (this pass)**: **Deferred.** The trait is consumed via
  `Arc<dyn ToolMaskPolicy>` injection at 4 call sites (`agent/mod.rs`,
  `dispatch/mod.rs`, `agent/tests.rs`, `task_deliver.rs` tests); a
  const-slice version saves ~6 LOC but loses the inline WHY-comments
  ("Story 1.13b: ...", "Story 2.14: ...") that explain each rule.
  L-severity; defer.
- **Pay down**: Phase 3+ if the mask rules grow non-trivially.

### ~~56. WHAT-only section dividers in `tools/builtin.rs`~~ — CLOSED in cross-phase hardening pass (`082688c`)
- **Origin**: REVIEW §4 Section A
- **Severity**: **Low**
- **What**: Five `// ===== name =====` dividers restated the
  `pub struct Name;` two lines below — pure WHAT noise.
- **Resolution**: Removed the 5 pure dividers. Kept the 2 dividers
  that carry actual WHY commentary (story 0.9 sandbox-tool subset
  rationale at line ~290; story 0.7 StubTool shape contract at
  line ~1017).

### ~~57. WHY-comments missing on Phase 0/1 constants~~ — CLOSED in cross-phase hardening pass (`becf4da`)
- **Origin**: REVIEW §4 Section A
- **Severity**: **Low**
- **What**: Five Phase 0/1 sites carried hard-coded constants without
  inline WHY: `agent/stuck.rs` 2/4 thresholds, `agent/diversity.rs`
  4-variant array, `llm/mod.rs` BIFROST_MASTER_KEY placeholder,
  `db/mod.rs` DbPool `Arc<Mutex<Connection>>` choice,
  `plan/render.rs` `* 3` chars-per-token heuristic.
- **Resolution**: Each site now carries a 2-5 line `///` or `//!`
  citing the relevant phase DEBT entry / ADR.

### 58. `pub` shrinkage + missing `///` summaries on Phase 0/1 types — partially closed (`66cdc3f`)
- **Origin**: REVIEW §4 Section B + H
- **Severity**: **Low**
- **What**: Phase 0/1 carried a handful of public types without
  `///` summaries (`RunRequest`, `RunResult`, `AgentRunner`,
  `AgentRunnerDeps` in `agent/mod.rs`) and a few `pub` items whose
  only callers are sibling modules in the same crate
  (`agent::build_messages`, `verifier::Worker::handle_request_with_watchdog`).
- **Status (`66cdc3f`)**: The four agent/mod.rs types now have
  one-line `///` summaries. `CheckpointManager::run` already had its
  Phase 1 baseline WHY-comment.
- **Open**: The `pub` → `pub(crate)` shrinkages on
  `build_messages` + `handle_request_with_watchdog`. Both have
  legitimate same-crate sibling callers; shrinking visibility would
  just shuffle keywords for zero observable change. Phase 3 housekeeping.

### 59. `GET /v1/sessions*` GET routes not loopback-gated — CLOSED (`18d472d`, same commit as #48)
- **Origin**: REVIEW §1 Section A
- **Severity**: **Medium**
- **What**: `/v1/sessions`, `/v1/sessions/:id`, `/v1/sessions/:id/events`,
  `/v1/sessions/:id/feature-list`, `/v1/sessions/:id/progress`
  exposed session inventory + event payloads + workspace-derived
  feature-list / progress files on non-loopback binds.
- **Resolution**: Same commit as #48 added the
  `ConnectInfo<SocketAddr>` extractor + `require_loopback(remote)?` to
  all five handlers. Test fixtures `tests/events.rs` boot helper
  migrated to `into_make_service_with_connect_info`;
  `tests/feature_list.rs` migrated from `app.oneshot(...)` to real
  TcpListener + reqwest pattern. New regression test
  `list_sessions_refuses_non_loopback_remote` covers the gate.

### 60. Phase 1 large-file split set
- **Origin**: REVIEW §4 Section F
- **Severity**: **Low**
- **What**: Beyond the Phase 2 review's existing
  `task_deliver.rs` (1082L), `notify/worker.rs` (621L),
  `channel/email/mod.rs` (621L) split candidates, cross-phase sweep
  surfaced four more borderline prod files: `agent/mod.rs` (725L),
  `sandbox/mod.rs` (715L), `verifier/worker.rs` (677L),
  `agent/init/mod.rs` (657L).
- **Pay down**: Bundle into a single Phase 3 polish PR alongside
  DEBT #52.

### 61. EventType `Knowledge` / `Datasource` / `Skill` reserved but unwired
- **Origin**: REVIEW §3 Section B
- **Severity**: **Low** (informational — Phase 3 territory)
- **What**: `EventType` enum + V002 CHECK constraint both carry 8
  variants. Three of them (`Knowledge`, `Datasource`, `Skill`) have
  no production emit path. Phase 3 Curator populates them.
- **Pay down**: Phase 3 — write the emit sites + add a one-line doc
  comment on the enum variants noting "Phase 3+ Curator emission".

### 62. `spec-check.sh` hard-coded tool count lacks phase-version gate
- **Origin**: REVIEW §3 Section E
- **Severity**: **Low**
- **What**: `scripts/spec-check.sh:65-72` hard-codes
  `expected=39` (38 unique tools + the `task_deliver` registration
  override). Correct at HEAD but detached from spec; Phase 3 adding a
  learning tool will silently fail the gate until manually updated.
- **Pay down**: Phase 3 housekeeping — extract to a const + cite the
  spec section + maintain alongside any tool-catalog change.

### 63. Frontend `pnpm test` is a passing stub
- **Origin**: REVIEW §3 Section E
- **Severity**: **Low**
- **What**: `frontend/package.json` test script exits 0 without
  running anything. Playwright lives at `pnpm test:e2e` and runs
  only under `workflow_dispatch`. The `just verify` chain → `pnpm
  test` is a no-op gate.
- **Pay down**: Phase 3 — when FE unit tests land, replace the
  stub. Until then, the `test:e2e` workflow_dispatch jobs are the
  load-bearing FE verification surface.

### 64. `tenant_id: None` 100% hardcoded — Phase 5 conversion meta-DEBT
- **Origin**: REVIEW §1 Section J + §2 tenant_id ceremony
- **Severity**: **Low** (informational — Phase 5 territory)
- **What**: Every production construction site that builds an
  `IntakeEvent`, `Deliverable`, `Brief`, `DeliveryEvent`, etc.
  passes `tenant_id: None`. The field exists as forward-compat for
  the Phase 5 NOT-NULL flip.
- **Pay down**: Phase 5 multi-user — single atomic commit that
  (a) drops `Option` wrap on the field, (b) updates all 55+
  construction sites, (c) updates DB load-paths, (d) wires the
  auth layer to fill the value at boundary.

---

## Categories quick-reference (same as Phase 0 / Phase 1)

| Severity | Meaning |
|---|---|
| **H** | Blocks the next phase's goals if not addressed |
| **M** | Will bite at scale or in a year, manageable today |
| **L** | Documentation / minor friction / one-line fix later |
