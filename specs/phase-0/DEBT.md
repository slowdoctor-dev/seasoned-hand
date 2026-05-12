# Phase 0 — Technical Debt Ledger

> Append-only list of shortcuts, stubs, simplifications, and deferred
> work introduced during Phase 0. Each entry: title, origin story,
> what was deferred, why, who pays it down, severity (L/M/H).
>
> Discipline: every Phase 0 story commit that incurs new debt updates
> this file in the same commit. When the user asks "what's left?",
> read this file first.

---

## Open debt (from stories 0.1–0.7)

### 1. `DbPool` is `Arc<Mutex<Connection>>`, single-writer
- **Origin**: story 0.3
- **Severity**: **Medium**
- **What**: A single `rusqlite::Connection` wrapped in `tokio::Mutex`. Every DB
  read/write serializes through one lock.
- **Why**: Phase 0 has 1 user, ≤5 concurrent sessions, dozens of writes/min.
  A real pool (`r2d2-sqlite`, `deadpool-sqlite`) adds dependency surface and
  configuration we didn't need yet.
- **Pay down**: Phase 1 — when story 0.27 E2E shows contention, OR when
  multi-tenant Phase 5 lands.
- **Detection**: criterion bench in story 0.4 will flag if append latency
  exceeds the §7 budget under simulated load.

### 2. EventStore trait does NOT have a `subscribe` method
- **Origin**: story 0.5 (spec drift)
- **Severity**: **Medium**
- **What**: Story 0.5's spec said the `EventStore` trait gains a `subscribe`
  method. Implementation put `subscribe` on `RedisPool` instead. The trait
  surface stays append-only at the type level (which is also nice).
- **Why**: Pub/sub is a connection-management concern, not a "store" concern.
  Mixing them complicates `dyn EventStore` (subscribe returns a non-trait-object
  type). Kept clean trait, exposed subscribe elsewhere.
- **Pay down**: Story 0.17 (WebSocket server) uses `RedisPool::subscribe`
  directly. If callers ever want polymorphism over (Redis | NATS | …)
  subscribers in Phase 4+, revisit then.

### 3. Tool stubs return generic `not_implemented` — no per-tool dispatch yet
- **Origin**: stories 0.6, 0.7
- **Severity**: **Low**
- **What**: 28 of 33 tools return `{ok:false, error:{kind:"not_implemented",
  message:"backend pending (see story X.Y)"}}`. The agent runtime (story 0.14)
  must distinguish "tool half-built" from "tool error" so the LLM doesn't loop
  forever retrying.
- **Why**: Story-by-story story ordering — backends fill in 0.8/0.9/0.14.
- **Pay down**: When stories 0.8, 0.9, 0.14 land, the corresponding
  `StubTool` entries get replaced with real impls and the count of stubs drops
  to 3 (the permanent Phase-0 stubs: `deploy_expose_port`,
  `deploy_apply_deployment`, `playbook_search`).

### ~~4. Tool catalog count is 33, architecture says "32"~~ ✅ resolved 2026-05-12 (story 0.27)
- ~~Origin: story 0.7~~
- ~~Resolved: spec-check.sh now asserts 33 (architecture's 32 + plan_advance
  per ADR-010); failed-gate if it ever drifts. Architecture §7 still reads
  "32" but with the proviso that plan_advance/plan_update are separately
  enumerated under ADR-010. Doc-only follow-up may reconcile the wording.~~

### 5. `async-trait` macro on `Tool` (Box-pin per call)
- **Origin**: story 0.6
- **Severity**: **Low**
- **What**: `Tool::invoke` uses `#[async_trait]` which boxes the returned
  future. One heap allocation per tool dispatch.
- **Why**: Needed for `Arc<dyn Tool>` registry. The bare `async fn in trait`
  doesn't make traits dyn-compatible.
- **Pay down**: When (if) Rust gets dyn-compatible async fn natively. Not
  expected in Phase 0.

### 6. `EventStore` trait uses `#[allow(async_fn_in_trait)]` and isn't dyn-compatible
- **Origin**: story 0.4
- **Severity**: **Low**
- **What**: Bare async fn in trait; only usable with concrete types
  (`Arc<SqliteEventStore>`, not `Arc<dyn EventStore>`). Couples
  `ToolContext.events` to the concrete sqlite impl.
- **Why**: Saves the Box-pin overhead since we only have one impl in Phase 0.
- **Pay down**: If a second EventStore impl ever appears (e.g., a NATS-backed
  one for distributed mode), switch to `#[async_trait]`.

### 7. WebSocket auth: **none** (Phase 0 = 127.0.0.1 only)
- **Origin**: story 0.2 / architecture §9
- **Severity**: **High when multi-user lands** / **Low today**
- **What**: The Axum server binds 127.0.0.1, no bearer tokens, no CORS
  controls. Any local process can hit the API.
- **Why**: Self-hosted single-user Phase 0. ADR-008 / architecture §9.
- **Pay down**: Phase 5 — multi-user. Bearer-token auth + per-session
  authorization.

### 8. Bifrost: no auth in Phase 0 (`BIFROST_MASTER_KEY` env unused)
- **Origin**: story 0.1
- **Severity**: **Low today** / **Medium at exposure time**
- **What**: Bifrost container bound to 127.0.0.1 without master-key
  enforcement. `BIFROST_MASTER_KEY` env var exists as forward-compat
  scaffolding but is unused.
- **Why**: localhost-only Phase 0.
- **Pay down**: Phase 5 — when Bifrost may be exposed beyond localhost,
  control plane must send the master key.

### 9. Bifrost smoke test partially blocked on missing real API keys
- **Origin**: story 0.1
- **Severity**: **Low** (test only, doc'd)
- **What**: `scripts/test-bifrost.sh` fail-fasts on missing
  `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`, so a fresh checkout without keys
  can't fully verify the gateway.
- **Why**: Real LLM calls need real keys; mocking Bifrost would require
  building a fake provider.
- **Pay down**: Phase 1 hardening — add a `--allow-no-keys` mode that
  skips cloud tests but verifies the gateway is at least healthy.

### 10. `cargo run` paths require `source $HOME/.cargo/env` in non-login shells
- **Origin**: story 0.2 / local-env quirk
- **Severity**: **Low** (env-specific)
- **What**: Rust toolchain installed at `$HOME/.cargo`, not on default PATH
  for non-login shells. Build commands need explicit env-sourcing.
- **Why**: rustup default install location; this WSL shell isn't a login
  shell.
- **Pay down**: `justfile` should add the env-sourcing automatically when
  story 0.27 wires real `just verify`.

### 11. Bifrost cost tracking is poll-based via `GET /cost`, not push
- **Origin**: story 0.1 (and architecture §4.4)
- **Severity**: **Low**
- **What**: Control plane will poll Bifrost's `/cost` endpoint after each
  tool call (story 0.16) rather than receiving a webhook.
- **Why**: Bifrost v1.5.0 doesn't ship a push callback path. Polling fits
  our "one tool per iteration" cadence and is simple.
- **Pay down**: Phase 1+ if Bifrost adds a push hook.

### 12. Redis live tests are `#[ignore]`'d by default
- **Origin**: story 0.5
- **Severity**: **Low**
- **What**: 3 pub/sub round-trip tests need a real Redis and are
  `#[ignore]`d. CI must explicitly run `cargo test -- --ignored` after
  bringing Redis up.
- **Why**: Avoided `testcontainers` dep weight; tests run cheap locally
  with `docker compose up -d redis`.
- **Pay down**: CI workflow story will add a Redis service container and
  run ignored tests there.

### 13. `Cargo.toml` `repository` URL points to `slowdoctor-dev/seasoned-hand`
- **Origin**: story 0.2
- **Severity**: **Low** (documentation)
- **What**: Hard-coded GitHub URL in workspace metadata. If the repo
  ever moves to an org (`/specs/06-roadmap/ROADMAP.md` mentions Phase 6
  considers org migration), update this.
- **Pay down**: Phase 6 if/when repo moves.

### 14. CI workflow not validated against new structure
- **Origin**: inherited from initial scaffold commit `a4b819f`
- **Severity**: **Medium**
- **What**: `.github/workflows/ci.yml` exists from day 0 but hasn't been
  exercised against the now-real Rust workspace + tests. Likely needs
  updates to install cargo, run clippy + fmt + test + spec-check.
- **Pay down**: Add a dedicated CI-fix story (or fold into story 0.27 E2E).

### 15. Sandbox per-session container needs `seccomp=unconfined`
- **Origin**: story 0.8
- **Severity**: **Medium** (security-tradeoff)
- **What**: AIO Sandbox requires `--security-opt seccomp=unconfined` for
  Chromium to sandbox processes internally. The container therefore has
  a wider host-level syscall surface than the default Docker policy.
- **Why**: Upstream `agent-infra/sandbox` README mandates it. Phase 0 is
  localhost-only single-user (architecture §9); the host-level risk is
  bounded by Docker user-namespacing + 127.0.0.1 binding.
- **Pay down**: Phase 1 hardening — consider a tailored seccomp profile
  that allows the Chromium-needed syscalls but no more. Or migrate to
  Firecracker microVMs (ADR-004 Alternative A) for enterprise tier.

### 16. Sandbox workspace cleanup is manual (orphan dirs accumulate)
- **Origin**: story 0.8
- **Severity**: **Low**
- **What**: `SandboxClient::destroy` removes the container but does NOT
  delete `{workspace_root}/{session_id}/`. If a session ends, its workspace
  dir lingers on disk indefinitely.
- **Why**: Workspaces may contain artifacts the user wants to download
  later. Deletion needs a retention policy.
- **Pay down**: Phase 1 — add a configurable workspace TTL + a cleanup
  job (cron or on-startup sweep).

### 17. Live sandbox lifecycle test is `#[ignore]`'d and pulls ~1 GB
- **Origin**: story 0.8
- **Severity**: **Low** (CI-time cost only)
- **What**: `live_create_inspect_destroy` requires Docker and pulls the
  full AIO Sandbox image. Not run in default `cargo test`.
- **Pay down**: CI workflow (item 14) should bring up Docker, prime the
  image cache once per CI run, then `cargo test -- --ignored sandbox::`.

### 19. Story 0.9 shipped representative sandbox-tool wiring (4 of 22)
- **Origin**: story 0.9
- **Severity**: **Medium**
- **What**: Of the 22 sandbox-backed tools that story 0.9 was supposed to
  wire, only 4 are real (`file_read`, `file_write`, `shell_exec`,
  `info_search_web`). The other 18 (file_str_replace, file_find_in_content,
  file_find_by_name, shell_view/wait/write_to_process/kill_process, 12
  browser_*) remain `StubTool`s.
- **Why**: Story 0.9 was sized at 3h; verifying the AIO Sandbox v1.0.0.152
  API contract for each tool (especially the browser action namespace which
  multiplexes into `/v1/browser/actions` with action-typed bodies) is a
  bigger discovery task than expected. The dispatcher + `sandbox_post`
  helper + 4 representative real tools demonstrate the wiring pattern is
  sound; the other 18 are mechanical follow-up work.
- **Pay down**: New story 0.9b — "wire remaining 18 sandbox tools" —
  uses the same `sandbox_post` helper. Should land before story 0.27 E2E
  if the E2E test exercises browsing (which it does — "find GitHub stars
  of FoundationAgents/OpenManus").

### 27. Frontend shipped on Next.js 16 (not 15 as architecture says)
- **Origin**: story 0.18
- **Severity**: **Low** (forward-compat drift)
- **What**: `create-next-app@latest` produced Next.js 16.2.6 / React 19.2.4 /
  Tailwind 4.3.0. Architecture §5.3 and BASELINE.md still say "Next.js 15".
- **Why**: Pinning at create-time would have meant rolling our own template;
  shipping latest is the lower-friction choice and matches Phase 0's
  "use upstream defaults" pattern.
- **Pay down**: Update ARCHITECTURE.md §1 + §5.3 + BASELINE.md §4 to read
  "Next.js 16" in a doc-only commit (no code change).

### ~~20. ToolDispatcher ships with no hooks registered~~ ✅ resolved 2026-05-12 (story 0.10)
- ~~Origin: story 0.9~~
- ~~Resolved by story 0.10: `EventEmittingHook` writes Action + Observation
  events for every dispatch; AppState::new registers it automatically.~~

### 21. Hook output-truncation path falls back to inline preview
- **Origin**: story 0.10
- **Severity**: **Low**
- **What**: When a tool output's JSON exceeds `INLINE_OUTPUT_LIMIT`
  (16 KB), `downsize_output()` replaces it with a `{preview, truncated:true}`
  marker. Architecture §3.4 specifies writing the full body to
  `/workspace/.observations/<call_id>.txt` via the sandbox and storing
  only a `file_ref` in the event.
- **Why**: The sandbox-file-write path needs the 18-of-22 sandbox tools
  (DEBT #19) plus a write helper. Defer until that wiring lands.
- **Pay down**: When DEBT #19 (story 0.9b) closes, add a
  `write_observation_file(call_id, content)` helper to the hook and replace
  the inline truncation with the file_ref path.

### 22. Capability table assumes Bifrost cloud aliases support tool calling
- **Origin**: story 0.13
- **Severity**: **Low**
- **What**: `built_in_capabilities()` treats `agent-primary` and
  `agent-fallback` as ToolCalling + Vision + LongContext aliases so the
  Phase 0 default router (`main -> agent-primary`) can pass startup
  validation even when Bifrost `/v1/models` returns aliases instead of
  provider model IDs.
- **Why**: Story 0.13 is a static startup gate only; real probing with a
  dummy tool call is explicitly out of scope because it costs money.
- **Pay down**: Phase 1 — resolve Bifrost aliases to their configured
  provider model IDs before checking capabilities, or perform a cheap
  provider-supported metadata probe if Bifrost exposes capability flags.

### 18. SandboxClient holds in-process handle cache — single-process assumption
- **Origin**: story 0.8
- **Severity**: **Medium**
- **What**: `Arc<RwLock<HashMap<String, SandboxHandle>>>`. If the control
  plane crashes and restarts, the cache is empty but real containers from
  prior runs may still exist. Currently relies on container *names* to
  recover (we can re-create via `destroy` + `create`).
- **Why**: Phase 0 has no persistence layer for runtime state beyond
  events. A real "sandbox registry" in SQLite or Redis is a future story.
- **Pay down**: Phase 1 — on startup, the SandboxClient could scan Docker
  for `seasoned-hand-sandbox-*` containers and rehydrate the cache.

### ~~23. Stuck detection only emits an audit marker~~ ✅ resolved 2026-05-12 (story 0.15)
- ~~Origin: story 0.14~~
- ~~Resolved by story 0.15: `agent::stuck::StuckTracker` injects a
  strategy-change prompt at 2 repeated responses and terminates the session
  as ERROR at 4 repeated responses.~~

### ~~24. Runner accepts cost caps but does not enforce them~~ ✅ resolved 2026-05-12 (story 0.16)
- ~~Origin: story 0.14~~
- ~~Resolved by story 0.16: the runner polls Bifrost `/cost`, increments
  `sessions.cost_cents`, and suspends the session with `Misc{kind:"cost_cap"}`
  when the configured cap is reached.~~

### 25. Plan tools remain callable stubs
- **Origin**: story 0.14
- **Severity**: **Medium**
- **What**: The runner seeds a baseline Plan event and exposes the registered
  plan tools to the LLM, but `plan_create`, `plan_advance`, and `plan_update`
  still return `not_implemented` instead of mutating a structured plan table.
- **Why**: ADR-010 full Plan Manager behavior is larger than the baseline
  ReAct loop; story 0.14 only needed sticky plan context and callable tool
  specs.
- **Pay down**: Follow-up Plan Manager story — wire the plan tools to the
  `plans` table, emit Plan events for each mutation, and make sticky context
  render the latest structured plan instead of raw events.

### 26. Cost deltas assume one active session per Bifrost instance
- **Origin**: story 0.16
- **Severity**: **Medium**
- **What**: `CostClient` reads Bifrost's aggregate `/cost` counter. The runner
  attributes each positive aggregate delta to the active session.
- **Why**: Phase 0 is single-user and expects one task at a time. Bifrost does
  not expose per-session/per-request attribution in the current contract.
- **Pay down**: Phase 1 — add per-request cost attribution if Bifrost exposes
  it, or isolate Bifrost accounting per session before concurrent sessions
  are allowed.

### 27. WS task_pause/task_resume/task_cancel are protocol stubs
- **Origin**: story 0.17
- **Severity**: **Medium**
- **What**: `/ws` accepts the three control commands and returns `ack`, but
  it does not yet coordinate real cancellation tokens or sandbox pause/resume
  semantics per session.
- **Why**: Phase 0 only needs envelope compatibility and task-create/resume
  flow; robust cooperative cancellation wiring is deferred.
- **Pay down**: Story 0.27 or Phase 1 — add per-session cancel tokens in
  runtime state and wire pause/resume/cancel to runner checkpoints + sandbox.

---

## Categories quick-reference

| Severity | Meaning | Examples |
|---|---|---|
| **H** | Blocks the next phase's goals if not addressed | Multi-user auth before Phase 5 |
| **M** | Will bite at scale or in a year, manageable today | DbPool single-writer, CI workflow drift |
| **L** | Documentation / minor friction / one-line fix later | Cost polling vs push, Cargo.toml repo URL |
