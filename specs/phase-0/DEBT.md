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

### 4. Tool catalog count is 33, architecture says "32"
- **Origin**: story 0.7
- **Severity**: **Low** (documentation / counting)
- **What**: `scripts/spec-check.sh` warns "Tool catalog has 33 tools, spec says
  32". The 33 = the architecture's 32 + `plan_advance` (or maybe the spec's
  "32" was always approximate — see story-0.7.md for the breakdown).
- **Why**: Architecture §7's "32" is internally inconsistent (counts
  `make_manus_page` differently than the §4.3 routing table). Picked a
  defensible 33.
- **Pay down**: One of the next architecture edits should reconcile the
  count and update §7 to 33 OR reduce the registry to 32. Low priority —
  the warning doesn't fail any gate.

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

---

## Categories quick-reference

| Severity | Meaning | Examples |
|---|---|---|
| **H** | Blocks the next phase's goals if not addressed | Multi-user auth before Phase 5 |
| **M** | Will bite at scale or in a year, manageable today | DbPool single-writer, CI workflow drift |
| **L** | Documentation / minor friction / one-line fix later | Cost polling vs push, Cargo.toml repo URL |
