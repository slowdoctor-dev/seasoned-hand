# Story 0.13 — Capability auto-detection

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: story 0.11 (LLM client), story 0.12 (slot router)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/01-architecture/ARCHITECTURE.md` §3 (12-slot routing), `/specs/phase-0/architecture.md` §4.4, ADR-003

---

## Goal

At server startup, verify that the `main` slot's resolved model
supports tool-calling — the architecture §4 HARD constraint. If it
doesn't, **fail fast** with a clear error rather than launching a
broken agent loop.

Phase 0 scope: probe `/v1/models` (via `LlmClient::list_models`),
match each slot's model against a capability table, and assert the
constraint. Other capabilities (vision, JSON mode, long context) are
scaffolded but not enforced.

## Acceptance criteria

- [ ] `seasoned-hand-core::capability` module with:
      - `Capability` enum: `ToolCalling`, `Vision`, `JsonMode`,
        `LongContext`, `Embedding`
      - `ModelCapabilities` struct: a set of `Capability` values plus
        `model_id`
      - `CapabilityProbe::new(client: LlmClient) -> Self`
      - `async fn probe_models() -> Result<HashMap<String, ModelCapabilities>, CapabilityError>`
      - `assert_main_supports_tool_calling(&router, &probed) -> Result<(), CapabilityError>`
- [ ] Capability lookup strategy: first check if Bifrost's `/v1/models`
      response includes capability fields (Phase 0 falls back to a
      hard-coded table when absent); then look up by model id substring
- [ ] Phase-0 hard-coded table (`built_in_capabilities()`):
      - `claude-*` → ToolCalling + Vision + LongContext
      - `gpt-4*` / `gpt-4o*` → ToolCalling + Vision + JsonMode + LongContext
      - `qwen*` (32b+) → ToolCalling
      - `llama3.1*`, `llama3.2:*b` where N ≥ 8 → ToolCalling
      - `llama3.2:3b` (story-0.1 default `local-fast`) → no
        ToolCalling capability (small models miss it)
      - `text-embedding-*` → Embedding only
      - Unknown → empty set
- [ ] `main` slot startup gate: if main's model isn't in the table AND
      Bifrost's `/models` response doesn't claim tool-calling, return
      `CapabilityError::MainLacksToolCalling { model }` — server bails
- [ ] Other slots: log a warning if a missing capability is implied by
      slot name (vision slot points at a non-vision model) but don't
      fail
- [ ] Unit tests:
      - `claude_sonnet_supports_tool_calling`
      - `gpt_4o_supports_tool_calling_and_vision`
      - `llama_3_2_3b_does_not_claim_tool_calling`
      - `unknown_model_has_empty_capabilities`
      - `probe_returns_empty_when_models_endpoint_unreachable` (uses
        an offline base_url; expects Err, doesn't panic)
      - `assert_main_passes_with_claude_sonnet`
      - `assert_main_fails_with_llama_3_2_3b`
- [ ] Server `main.rs` runs the probe at startup; on
      `MainLacksToolCalling` returns nonzero exit with a clear message
- [ ] `cargo clippy / fmt / test / spec-check` all pass

## Non-goals

- Live updating the capability table at runtime (Phase 1)
- Per-slot startup enforcement (Phase 1; only `main` is hard-gated in
  Phase 0)
- Calling the model with a dummy tool to verify (real call costs $).
  Capability checks are static.
- Vision/JSON-mode runtime probing (Phase 1+)

---

## Implementation steps

### 1. Types

```rust
// crates/seasoned-hand-core/src/capability/mod.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    ToolCalling,
    Vision,
    JsonMode,
    LongContext,
    Embedding,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub model_id: String,
    pub capabilities: HashSet<Capability>,
}

#[derive(Debug, Error)]
pub enum CapabilityError {
    #[error("llm: {0}")]
    Llm(#[from] crate::llm::LlmError),
    #[error("main slot model '{model}' does not support tool-calling — architecture §4 hard constraint")]
    MainLacksToolCalling { model: String },
}

pub struct CapabilityProbe {
    client: LlmClient,
}
```

### 2. Built-in capability table

```rust
pub fn built_in_capabilities(model_id: &str) -> ModelCapabilities {
    let mut caps = HashSet::new();
    let id = model_id.to_lowercase();

    if id.starts_with("claude-") {
        caps.extend([Capability::ToolCalling, Capability::Vision, Capability::LongContext]);
    } else if id.starts_with("gpt-4") {
        caps.extend([
            Capability::ToolCalling, Capability::Vision,
            Capability::JsonMode, Capability::LongContext,
        ]);
    } else if id.starts_with("qwen") {
        // qwen2.5:32b, qwen3, etc.
        caps.insert(Capability::ToolCalling);
    } else if id.starts_with("llama3.1") {
        caps.insert(Capability::ToolCalling);
    } else if id.starts_with("llama3.2:") {
        // size suffix; only 8B+ get tool-calling claim in Phase 0
        if let Some(size) = id.strip_prefix("llama3.2:").and_then(parse_size_b) {
            if size >= 8 { caps.insert(Capability::ToolCalling); }
        }
    } else if id.starts_with("text-embedding-") {
        caps.insert(Capability::Embedding);
    }
    // unknown → empty caps

    ModelCapabilities { model_id: model_id.to_string(), capabilities: caps }
}

fn parse_size_b(s: &str) -> Option<u32> {
    // "3b", "7b", "8b" -> 3, 7, 8
    s.strip_suffix('b').and_then(|n| n.parse::<u32>().ok())
}
```

### 3. Probe + assertion

```rust
impl CapabilityProbe {
    pub fn new(client: LlmClient) -> Self { Self { client } }

    /// Call /v1/models, merge with built-in table.
    /// Bifrost models list returns model ids only (no capability flags
    /// in v1.5.0); we use the list to confirm the model is *available*,
    /// then look up capabilities from built_in_capabilities.
    pub async fn probe(&self) -> Result<HashMap<String, ModelCapabilities>, CapabilityError> {
        let models = self.client.list_models().await?;
        let mut map = HashMap::new();
        for m in models {
            let caps = built_in_capabilities(&m.id);
            map.insert(m.id.clone(), caps);
        }
        Ok(map)
    }

    pub fn assert_main_supports_tool_calling(
        router: &SlotRouter,
        probed: &HashMap<String, ModelCapabilities>,
    ) -> Result<(), CapabilityError> {
        let main = router.resolve(SlotName::Main);
        let caps = probed
            .get(&main.model)
            .cloned()
            .unwrap_or_else(|| built_in_capabilities(&main.model));
        if !caps.capabilities.contains(&Capability::ToolCalling) {
            return Err(CapabilityError::MainLacksToolCalling { model: main.model.clone() });
        }
        Ok(())
    }
}
```

### 4. Server startup

`main.rs`:

```rust
let llm = LlmClient::new(/* router.resolve(Main).base_url */, /* api_key */);
let probe = CapabilityProbe::new(llm.clone());
let probed = probe.probe().await.unwrap_or_default();  // tolerate offline; rely on built-in
CapabilityProbe::assert_main_supports_tool_calling(&router, &probed)?;
```

If Bifrost is offline, the empty `probed` map falls through to the
built-in lookup. Hard constraint still enforced.

### 5. AppState

Carries `capabilities: Arc<HashMap<String, ModelCapabilities>>` for
later stories (story 0.14 may consult it when building tool-spec
arrays). Phase 0 just stores it.

### 6. Tests

Cover the matrix above, plus probe-against-unreachable, plus the
assert function for two model paths.

---

## Files changed

- `crates/seasoned-hand-core/src/lib.rs` (`pub mod capability`)
- `crates/seasoned-hand-core/src/capability/mod.rs` (new)
- `crates/seasoned-hand-core/src/capability/tests.rs` (new)
- `crates/seasoned-hand-server/src/lib.rs` (AppState.capabilities)
- `crates/seasoned-hand-server/src/main.rs` (probe + assert)
- `crates/seasoned-hand-server/tests/healthz.rs` + `events.rs` (update
  AppState construction)

---

## Spec references

- `/specs/01-architecture/ARCHITECTURE.md` §3 (12-slot routing), §4
  (one-tool-per-iteration HARD constraint at `tool_choice="required"`
  level — predicated on the model supporting tool-calling)
- `/specs/01-architecture/decisions/ADR-003-12-slot-model-routing.md`
- `/specs/phase-0/architecture.md` §4.4 (capability detection at startup)

---

## Commit message

```
feat(phase-0): story 0.13 - capability auto-detection at startup

- seasoned-hand-core::capability with Capability enum
  (ToolCalling/Vision/JsonMode/LongContext/Embedding),
  ModelCapabilities struct, CapabilityProbe over LlmClient
- built_in_capabilities() Phase-0 hard-coded lookup table:
  claude-* → tool+vision+long, gpt-4* → +jsonmode, qwen* → tool,
  llama3.1* → tool, llama3.2:>=8b → tool, llama3.2:3b → none,
  text-embedding-* → embedding
- Probe merges Bifrost /v1/models response with built-in table
  (Bifrost v1.5.0 doesn't expose capability flags)
- assert_main_supports_tool_calling: hard error if main's model
  isn't in the table or Bifrost doesn't claim it (architecture §4)
- Server main.rs runs the probe at startup; falls back to built-in
  if Bifrost unreachable; fails fast on MainLacksToolCalling
- AppState gains capabilities: Arc<HashMap<String, ModelCapabilities>>
  for later stories
- N unit tests cover the matrix + assertion both passing and failing paths
- cargo clippy / fmt / test / spec-check all pass

refs: /specs/phase-0/stories/story-0.13.md
```

---

## Notes for next story (0.14)

- `AppState.capabilities` available; story 0.14 (agent runner) consults
  it when building tool-spec arrays for the LLM (skip vision-only tools
  when the model can't see images, etc.) — though Phase 0 only ships
  text tools so this is reserve infrastructure
- The hard-coded table is intentionally short and Phase-0; Phase 1
  adds runtime capability tests (try a dummy tool call to confirm)
