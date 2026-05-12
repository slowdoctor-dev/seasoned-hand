# Story 0.6 — Tool trait + 5 simplest tools

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: story 0.4 (Event Stream API), story 0.5 (Redis pub/sub)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/phase-0/architecture.md` §4.3 (Tool dispatcher, ToolContext, 5-backend routing table)

---

## Goal

Define the `Tool` trait and `ToolContext`, and ship the 5 simplest tools
(no sandbox required): `message_notify_user`, `message_ask_user`, `idle`,
`sop_read` (stub), `glossary_lookup` (stub). Establish the static
registry + JSON schema surface that the agent loop and dispatcher build on.

## Acceptance criteria

- [ ] `seasoned-hand-core::tools` module exposing:
      - `Tool` trait: `name()`, `description()`, `schema()`, `invoke(args, ctx)`
      - `ToolContext` carrying handles to backend clients (Frontend bus
        + event store; sandbox/search/deploy added in later stories)
      - `ToolError` enum (Invalid args, Backend, Internal)
      - `ToolOutput` struct: `{ ok: bool, output: Value, file_ref: Option<String>, error: Option<…> }`
      - `register_builtin_tools()` returns a `HashMap<&'static str, Arc<dyn Tool>>`
        with the 5 Phase-0 tools wired in
- [ ] Five tools implemented:
      - `message_notify_user(content: string)` — emits `Message`
        event with `ui:"notify"`, role:"assistant"; returns `{ok:true}`
      - `message_ask_user(content: string)` — emits `Message` event
        with `ui:"ask"`; returns `{ok:true, call_id:<event id>}`
        (agent loop blocks until a `user_response` arrives — that
        wiring is story 0.14, not here; this tool just emits the event)
      - `idle()` — returns `{ok:true, signal:"task_complete"}`;
        the agent loop interprets this in story 0.14
      - `sop_read(id: string)` — **stub**: returns
        `{ok:false, error:{kind:"not_implemented", phase:"3+"}}`
      - `glossary_lookup(term: string)` — **stub**: same shape as `sop_read`
- [ ] Each tool's `schema()` returns a valid JSON Schema (subset
      compatible with OpenAI function-call schema): `{type:"object",
      properties:{…}, required:[…]}` — schemas validated at startup
      via `serde_json::from_value::<serde_json::Value>`
- [ ] `register_builtin_tools()` returns a registry with exactly 5 entries
- [ ] Unit tests cover: each tool's invoke succeeds with valid args,
      rejects missing/typed-wrong args, emits the correct event type
      (assert by querying the event store after invocation), the registry
      contains 5 distinct names
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo fmt --check` passes
- [ ] `cargo test --workspace` passes
- [ ] `./scripts/spec-check.sh` passes

## Non-goals

- Tool dispatcher with hooks (story 0.9 + 0.10)
- Sandbox-backed tools (stories 0.7, 0.8)
- Search / deploy tools (stories 0.7, 0.9)
- Plan tools (folded into story 0.14)
- Tool calling from a live LLM (story 0.14)
- Architecture §4.3's "spec-check counts 32 tools" check stays at the
  current "warn only" behavior — 5 tools is fine for now

---

## Implementation steps

### 1. Types — `tools/mod.rs`

```rust
//! Tool catalog: 32 tools defined in architecture §4.3.
//! refs: /specs/phase-0/architecture.md §4.3, §7 (tool catalog)

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use serde_json::Value;

use crate::events::EventStore;

#[derive(Debug, Serialize)]
pub struct ToolOutput {
    pub ok: bool,
    pub output: Value,
    pub file_ref: Option<String>,
    pub error: Option<ToolErrorPayload>,
}

#[derive(Debug, Serialize)]
pub struct ToolErrorPayload {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    #[error("backend error: {0}")]
    Backend(String),
    #[error("not implemented (deferred to {0})")]
    NotImplemented(&'static str),
    #[error("internal: {0}")]
    Internal(String),
}

#[derive(Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub events: Arc<dyn EventStore>,
    // sandbox / search / deploy backends added in later stories
}

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn schema(&self) -> Value;
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError>;
}

pub mod builtin;

pub fn register_builtin_tools() -> HashMap<&'static str, Arc<dyn Tool>> {
    builtin::all()
}

#[cfg(test)]
mod tests;
```

Add `async-trait = "0.1"` to seasoned-hand-core's `[dependencies]`
(already in workspace.dependencies? Check; otherwise add).

### 2. Five tools — `tools/builtin.rs`

Each tool is a small unit struct implementing `Tool`. Example:

```rust
use serde_json::{json, Value};
use async_trait::async_trait;

use crate::events::{EventStore, EventType, NewEvent};
use super::{Tool, ToolContext, ToolError, ToolOutput};

pub struct MessageNotifyUser;

#[async_trait]
impl Tool for MessageNotifyUser {
    fn name(&self) -> &'static str { "message_notify_user" }
    fn description(&self) -> &'static str {
        "Send an informational message to the user. Fire-and-forget; the agent loop does not wait for a reply."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "Message body shown to the user." }
            },
            "required": ["content"],
            "additionalProperties": false,
        })
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let content = args.get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'content' string".into()))?
            .to_string();
        let event = ctx.events.append(NewEvent {
            session_id: ctx.session_id.clone(),
            event_type: EventType::Message,
            source: format!("tool:{}", self.name()),
            data: json!({"role": "assistant", "content": content, "ui": "notify"}),
        }).await.map_err(|e| ToolError::Backend(e.to_string()))?;
        Ok(ToolOutput {
            ok: true,
            output: json!({"event_id": event.id}),
            file_ref: None,
            error: None,
        })
    }
}
```

Repeat the pattern for the other 4. `register_builtin_tools()` returns:

```rust
pub fn all() -> HashMap<&'static str, Arc<dyn Tool>> {
    let mut map: HashMap<&'static str, Arc<dyn Tool>> = HashMap::new();
    map.insert("message_notify_user", Arc::new(MessageNotifyUser));
    map.insert("message_ask_user",    Arc::new(MessageAskUser));
    map.insert("idle",                Arc::new(Idle));
    map.insert("sop_read",            Arc::new(SopRead));
    map.insert("glossary_lookup",     Arc::new(GlossaryLookup));
    map
}
```

### 3. Tests — `tools/tests.rs`

Exercise each tool against an in-memory event store via the existing
`SqliteEventStore::new(pool)` fixture (no Redis required for tool tests
— append falls back to in-memory). For tools that emit events, assert
the resulting Event matches the expected type + source + payload.

Test cases:
- Registry has exactly 5 keys
- Each tool's `schema()` parses as JSON Schema (basic shape check)
- `message_notify_user` rejects missing `content`
- `message_notify_user` happy path emits Message event with ui=notify
- `message_ask_user` happy path emits Message event with ui=ask
- `idle` happy path returns task_complete signal
- `sop_read` returns NotImplemented for any input
- `glossary_lookup` returns NotImplemented for any input

---

## Files changed

- `crates/seasoned-hand-core/Cargo.toml` (add `async-trait = "0.1"`)
- `crates/seasoned-hand-core/src/lib.rs` (`pub mod tools`)
- `crates/seasoned-hand-core/src/tools/mod.rs` (new)
- `crates/seasoned-hand-core/src/tools/builtin.rs` (new)
- `crates/seasoned-hand-core/src/tools/tests.rs` (new)

---

## Spec references

- `/specs/phase-0/architecture.md` §4.3 (Tool dispatcher, ToolContext)
- `/specs/phase-0/architecture.md` §7 (32-tool catalog)
- `/specs/01-architecture/ARCHITECTURE.md` §7 (full Manus tool list)

---

## Commit message

```
feat(phase-0): story 0.6 - Tool trait + 5 simplest tools

- seasoned-hand-core::tools module with Tool trait, ToolContext,
  ToolError, ToolOutput, ToolErrorPayload
- 5 tools implemented: message_notify_user, message_ask_user, idle,
  sop_read (stub: NotImplemented deferred to Phase 3+),
  glossary_lookup (stub)
- register_builtin_tools() returns Arc<dyn Tool> registry
- Each tool exposes name, description, JSON Schema (OpenAI-compatible
  function-call format)
- 8 unit tests cover registry, schemas, happy paths, missing-arg
  rejection, stub responses
- cargo clippy / fmt / test / spec-check all pass

refs: /specs/phase-0/stories/story-0.6.md
```

---

## Notes for next story (0.7)

- Tool trait surface stable; story 0.7 adds the remaining 27 tools in
  batches grouped by backend (File/Shell/Browser via sandbox, Search,
  Deploy stub, plus the 3 Internal stubs already partially present here)
- `ToolContext` will grow `sandbox`, `search`, `deploy` fields in
  story 0.7+
