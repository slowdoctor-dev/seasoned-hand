# Story 0.9 — Tool dispatcher (5-backend routing + sandbox + search + hooks scaffold)

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: stories 0.4 (events), 0.6 (Tool trait), 0.7 (33 tools), 0.8 (sandbox client)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/phase-0/architecture.md` §4.3 (Tool dispatcher, ToolContext, routing table)

---

## Goal

Replace 22 of the 28 stubs from story 0.7 with real backend-routed
implementations. Specifically:

- **File (5)** + **Shell (5)** + **Browser (12)** = 22 tools routed to
  the AIO Sandbox HTTP API via the `SandboxClient::api_url`.
- **Search (1)** routed to Brave Search (default) or Tavily (configurable).
- The `deploy_*` (2), `playbook_search` (1), and `plan_*` (2) stubs stay
  as stubs in this story.

Also: ship the dispatcher itself — a thin layer above the tool registry
that:
1. Adds a `ToolDispatcher` newtype holding the registry + a
   `ToolContext` factory.
2. Adds the `sandbox` and `search` fields to `ToolContext`.
3. Wraps `tool.invoke(args, &ctx)` with **PreToolUse / PostToolUse /
   PostToolUseFailure** hook hooks — but the hooks bodies are deferred
   to story 0.10. This story ships the hook trait and the call sites.

## Acceptance criteria

- [ ] `seasoned-hand-core::dispatch` module with `ToolDispatcher`:
      - `new(registry, ctx_factory)`
      - `async fn dispatch(&self, session_id, tool_name, args) -> ToolOutput`
      - hook call sites for pre/post/failure (story 0.10 fills bodies)
- [ ] `ToolContext` extended with `sandbox: Arc<SandboxClient>` and
      `search: Arc<SearchClient>` fields (Phase 0 wires both)
- [ ] 22 sandbox-backed tool stubs replaced with real impls that
      forward to `ctx.sandbox.get(session_id).api_url + path` via reqwest.
      Path mapping example: `file_read` → `POST <api_url>/v1/file/read`,
      `shell_exec` → `POST <api_url>/v1/shell/exec`, etc. (Exact paths
      verified against AIO Sandbox v1.0.0.152 docs during impl.)
- [ ] If the session has no sandbox yet, those tools return
      `ToolError::Backend("sandbox not ready for session <id>")`
- [ ] `info_search_web` calls `https://api.search.brave.com/res/v1/web/search?q=...`
      with header `X-Subscription-Token: $BRAVE_API_KEY`. If
      `BRAVE_API_KEY` is unset, falls back to a stub `{ok:false,
      error:{kind:"missing_api_key"}}` rather than crashing.
- [ ] Server `AppState::new(db, redis, sandbox, search)` updated;
      `main.rs` builds both clients from env vars and passes them through.
- [ ] Workspace dir for sandbox = `${SANDBOX_WORKSPACE_HOST:-./data/workspaces}`
- [ ] Unit tests cover: dispatcher resolves tools by name; missing tool
      returns `ToolError::InvalidArgs`-ish 404; missing-sandbox path for
      sandbox-backed tools; missing-BRAVE_API_KEY for search
- [ ] Integration test: a full dispatch round-trip for `info_search_web`
      using a mocked HTTP server (or `#[ignore]`'d real Brave call)
- [ ] `cargo clippy / fmt / test / spec-check` all pass

## Non-goals

- Real PreToolUse / PostToolUse / PostToolUseFailure hook bodies (story 0.10)
- LLM client + slot router (stories 0.11–0.13)
- Agent runner loop (story 0.14)
- Frontend bus / WebSocket emit from dispatcher (story 0.17)
- Verifier (Phase 1)

---

## Implementation steps

### 1. Search client

`crates/seasoned-hand-core/src/search/mod.rs`:

```rust
//! Web search backends. Phase 0 ships Brave; Tavily comes later.
//! refs: /specs/phase-0/architecture.md §4.3

use serde::Deserialize;
use thiserror::Error;

#[derive(Clone)]
pub struct SearchClient {
    inner: reqwest::Client,
    provider: SearchProvider,
}

#[derive(Clone)]
pub enum SearchProvider {
    Brave { api_key: Option<String> },
    Tavily { api_key: Option<String> },
}

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("missing api key for provider {0}")]
    MissingApiKey(&'static str),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("parse: {0}")]
    Parse(String),
}

#[derive(Debug, Deserialize, Clone)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

impl SearchClient {
    pub fn new(provider: SearchProvider) -> Self {
        Self {
            inner: reqwest::Client::new(),
            provider,
        }
    }

    pub async fn web_search(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchHit>, SearchError> {
        // Brave-first impl. Tavily lands in Phase 1.
        match &self.provider {
            SearchProvider::Brave { api_key: None } => Err(SearchError::MissingApiKey("brave")),
            SearchProvider::Brave { api_key: Some(key) } => self.brave(query, max_results, key).await,
            SearchProvider::Tavily { .. } => Err(SearchError::MissingApiKey("tavily-deferred")),
        }
    }

    async fn brave(&self, query: &str, max: usize, key: &str) -> Result<Vec<SearchHit>, SearchError> {
        // Implementation in the actual story.
        todo!()
    }
}
```

(Actual brave() body in implementation: GET
`https://api.search.brave.com/res/v1/web/search?q=<urlencoded>&count=<n>`
with `X-Subscription-Token` header; deserialize `data.web.results[]`.)

### 2. Sandbox-backed tool helper

`tools/sandbox_helper.rs` (or inline in `builtin.rs`):

```rust
async fn sandbox_call(
    ctx: &ToolContext,
    path: &str,
    body: serde_json::Value,
) -> Result<ToolOutput, ToolError> {
    let handle = ctx
        .sandbox
        .get(&ctx.session_id)
        .await
        .ok_or_else(|| ToolError::Backend(format!("sandbox not ready for session {}", ctx.session_id)))?;
    let url = format!("{}{path}", handle.api_url);
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| ToolError::Backend(e.to_string()))?;
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| ToolError::Backend(e.to_string()))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    Ok(ToolOutput {
        ok: status.is_success(),
        output: v,
        file_ref: None,
        error: if status.is_success() { None } else { Some(ToolErrorPayload { kind: "http".into(), message: status.to_string() }) },
    })
}
```

Replace the 22 stubs with `pub struct FileRead;` etc. impls that build the
right path + body + call `sandbox_call`. AIO Sandbox v1.0.0.152's actual
API paths must be verified during implementation — likely:

- `POST /v1/file/read     { path }`
- `POST /v1/file/write    { path, content }`
- `POST /v1/shell/exec    { command, cwd? }`
- `POST /v1/browser/navigate { url }`
- ... etc.

If the upstream API uses different naming, document the actual paths in
the story commit and adjust this table.

### 3. Hook trait scaffold

`dispatch/hooks.rs`:

```rust
#[async_trait::async_trait]
pub trait Hook: Send + Sync {
    async fn pre(&self, name: &str, args: &Value, ctx: &ToolContext);
    async fn post(&self, name: &str, args: &Value, out: &ToolOutput, ctx: &ToolContext);
    async fn failure(&self, name: &str, args: &Value, err: &ToolError, ctx: &ToolContext);
}
```

Default `NoopHook` provided. Story 0.10 wires a real `EventEmittingHook`.

### 4. Dispatcher

```rust
pub struct ToolDispatcher {
    registry: HashMap<&'static str, Arc<dyn Tool>>,
    hooks: Vec<Arc<dyn Hook>>,
}

impl ToolDispatcher {
    pub fn new(registry: HashMap<&'static str, Arc<dyn Tool>>) -> Self { ... }
    pub fn with_hook(mut self, hook: Arc<dyn Hook>) -> Self { ... }
    pub async fn dispatch(&self, ctx: &ToolContext, tool_name: &str, args: Value) -> ToolOutput { ... }
}
```

`dispatch` flow:
1. Resolve `tool_name` → `Arc<dyn Tool>`. If absent: return
   `ToolOutput { ok:false, error: Some({kind:"unknown_tool"}) }`.
2. For each hook: `hook.pre(name, &args, ctx).await`.
3. Call `tool.invoke(args.clone(), ctx).await`.
4. On Ok: each hook `.post(...).await`. Return.
5. On Err: each hook `.failure(...).await`. Wrap as ToolOutput
   `{ok:false, error:{kind:"<err type>"}}`. Return.

### 5. Server wiring

Update `main.rs` to:
- Read `BRAVE_API_KEY`, `TAVILY_API_KEY`, `AIO_SANDBOX_IMAGE`,
  `SANDBOX_WORKSPACE_HOST` from env.
- Build `SandboxClient`, `SearchClient`, pass into `AppState::new(...)`.

`AppState` gains fields. Existing tests that call `AppState::new(pool,
redis)` need updating to `AppState::new(pool, redis, sandbox, search)`.

### 6. Tests

- dispatcher_unknown_tool returns `ok:false, error.kind:"unknown_tool"`
- dispatcher_routes_to_real_tool (use `message_notify_user` which is
  already real)
- sandbox_tool_returns_not_ready_when_session_has_no_container
- search_returns_missing_api_key_when_unset
- search_returns_results_against_mocked_http (use `wiremock`)

---

## Files changed

- `crates/seasoned-hand-core/Cargo.toml` (add `wiremock` dev-dep if used)
- `crates/seasoned-hand-core/src/lib.rs` (`pub mod dispatch`, `pub mod search`)
- `crates/seasoned-hand-core/src/search/mod.rs` (new)
- `crates/seasoned-hand-core/src/dispatch/mod.rs` (new)
- `crates/seasoned-hand-core/src/dispatch/hooks.rs` (new — scaffold)
- `crates/seasoned-hand-core/src/tools/mod.rs` (modify — ToolContext fields)
- `crates/seasoned-hand-core/src/tools/builtin.rs` (modify — 22 stubs → real)
- `crates/seasoned-hand-core/src/tools/tests.rs` (modify — update fixture)
- `crates/seasoned-hand-server/src/lib.rs` (AppState new fields)
- `crates/seasoned-hand-server/src/main.rs` (env reading)
- `crates/seasoned-hand-server/tests/events.rs` (modify — AppState construction)
- `crates/seasoned-hand-server/tests/healthz.rs` (modify — same)
- `specs/phase-0/DEBT.md` (any new entries)

---

## Spec references

- `/specs/phase-0/architecture.md` §4.3 routing table
- `/specs/01-architecture/ARCHITECTURE.md` §7 tool catalog
- `/specs/00-philosophy/PRINCIPLES.md` #10 failure-tolerant

---

## Commit message

```
feat(phase-0): story 0.9 - tool dispatcher (5-backend routing + sandbox + search + hook scaffold)

- seasoned-hand-core::dispatch with ToolDispatcher: registry-based
  name resolution + Hook trait scaffold (Pre/Post/Failure) — actual
  hook bodies land in story 0.10
- ToolContext extended: sandbox, search fields
- seasoned-hand-core::search with SearchClient (Brave default, Tavily
  enum reserved); missing-api-key path returns a clean ToolError
  rather than crashing (PRINCIPLE #10)
- 22 sandbox-backed tools (5 file + 5 shell + 12 browser) replaced
  from stubs to real reqwest calls against SandboxClient.api_url
- info_search_web wired to Brave
- AppState gains sandbox + search fields; main.rs reads env
- N tests: unknown tool, missing sandbox, search missing key,
  mocked Brave round-trip (wiremock)
- cargo clippy / fmt / test / spec-check all pass

refs: /specs/phase-0/stories/story-0.9.md
```

---

## Notes for next story (0.10)

- Dispatcher's hook call sites exist; story 0.10 implements
  `EventEmittingHook` that writes Action (pre), Observation (post),
  and an Observation-with-error (failure) events into the event stream
- Story 0.10 also wires the hook into `main.rs` AppState build
