# Story 1.7 — Bifrost alias → provider model-ID resolution + capability fallback (close DEBT #22)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: none
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-0/DEBT.md` #22 (what this closes),
> `/specs/phase-1/architecture.md` §6 row "Capability table fallback",
> §4.4 (Bifrost interface), `/specs/phase-0/stories/story-0.13.md`
> (capability auto-detect).

---

## Goal

Replace Phase 0's hardcoded "Bifrost cloud aliases support tool calling"
assumption with a real resolution path: at startup, query Bifrost
`/v1/models` for each slot's alias, read the underlying provider model ID,
and look up tool-calling support in a static capability table. After this
story, the agent runtime knows the *actual* provider model behind each
slot — required for story 1.8's `verifier ≠ main` resolved-model-ID gate.

## Acceptance criteria

- [ ] `seasoned-hand-core::router::capability::Resolver` exposes
      `async fn resolve_slot(slot: SlotName) -> Result<ResolvedSlot,
      RouterError>` returning
      `ResolvedSlot { slot, alias: String, provider_model_id: String,
      capabilities: CapabilityFlags }`.
- [ ] `CapabilityFlags { tool_calling: bool, json_mode: bool, vision: bool }`
      sourced from a static table at
      `crates/seasoned-hand-core/src/router/capability/table.rs`. Table
      entries cover at minimum: `claude-sonnet-4-6`, `claude-opus-4-7`,
      `claude-haiku-4-5`, `gpt-5.1`, `gpt-5.3-codex`, `llama3.2:3b`. An
      unrecognised model ID returns `CapabilityFlags::unknown()` — all
      flags `None` (tri-state: `Some(true)`, `Some(false)`, `None` = unknown).
- [ ] Resolution is **non-fatal at startup** for non-main slots: a slot
      whose alias does not resolve logs a warning and is recorded as
      `Unavailable`. The `main` slot remains hard-required (Phase 0
      behavior preserved).
- [ ] `SlotRouter::resolve_main()` keeps its existing API; the new
      `Resolver` is exposed via `SlotRouter::resolver() ->
      &Arc<Resolver>` for story 1.8's use.
- [ ] Phase 0 DEBT #22 entry struck through with date + commit ref.
- [ ] Tests:
      - `resolver_returns_capabilities_for_claude_sonnet_4_6` — pure
        unit on the table.
      - `resolver_returns_unknown_for_unrecognised_model` — confirms
        tri-state.
      - `resolve_slot_against_bifrost_mock` — wiremock'd Bifrost
        `/v1/models` returns a JSON model list; `resolve_slot(Main)`
        returns the right `provider_model_id` + capability flags.
      - `non_main_slot_unavailable_is_warning_not_error` — Bifrost
        returns 404 for a non-main slot's alias; startup completes; slot
        recorded as `Unavailable`.
      - `main_slot_unresolvable_is_startup_error` — `resolve_slot(Main)`
        Errs; server build returns the error (existing Phase 0 behavior
        preserved).

## Non-goals

- Auto-updating the capability table from Bifrost — table is hand-curated;
  Phase 4+ may grow this into a discovery mechanism.
- Probing actual capability at runtime by trial calls — out of scope;
  Bifrost is the source-of-truth for alias→model mapping, the table is
  the source-of-truth for what each model supports.
- Vision / json-mode probing — capability bits are recorded but only
  `tool_calling` is *consumed* in Phase 1. Story 1.8 uses
  `provider_model_id`.

## Implementation steps

### 1. Module

```
crates/seasoned-hand-core/src/router/capability/
  mod.rs       — Resolver, ResolvedSlot, CapabilityFlags
  table.rs     — static table + lookups
  tests.rs
```

### 2. Capability table

```rust
// table.rs
pub fn capabilities_for(model_id: &str) -> CapabilityFlags {
    use CapabilityFlags as F;
    match model_id {
        "claude-sonnet-4-6"     => F::all_yes_with(F::no_vision_unless_specified()),
        "claude-opus-4-7"       => F::all_yes(),
        "claude-haiku-4-5"      => F::all_yes(),
        "gpt-5.1"               => F::all_yes(),
        "gpt-5.3-codex"         => F::tool_calling_only(),
        "llama3.2:3b"           => F { tool_calling: Some(true),  json_mode: Some(true),  vision: Some(false) },
        _ => F::unknown(),
    }
}
```

The exact list is illustrative; pick the model IDs the team currently
runs through Bifrost. The table is *intentionally* small in Phase 1 —
Phase 4 may grow it.

### 3. Resolver

```rust
// mod.rs
pub struct Resolver {
    bifrost_base_url: String,
    http: reqwest::Client,
    slot_aliases: HashMap<SlotName, String>,
}

impl Resolver {
    pub async fn resolve_slot(&self, slot: SlotName) -> Result<ResolvedSlot, RouterError> {
        let alias = self.slot_aliases.get(&slot)
            .ok_or(RouterError::SlotNotConfigured(slot))?
            .clone();
        let url = format!("{}/v1/models/{}", self.bifrost_base_url.trim_end_matches('/'), alias);
        let resp = self.http.get(&url).send().await
            .map_err(|e| RouterError::Resolution { slot, source: e.into() })?;
        if !resp.status().is_success() {
            return Err(RouterError::AliasNotFound { slot, alias, status: resp.status() });
        }
        #[derive(Deserialize)]
        struct ModelInfo { id: String, #[serde(default)] owned_by: Option<String> }
        let info: ModelInfo = resp.json().await.map_err(|e| RouterError::Resolution {
            slot, source: e.into() })?;
        let caps = capabilities_for(&info.id);
        Ok(ResolvedSlot { slot, alias, provider_model_id: info.id, capabilities: caps })
    }
}
```

Note: Bifrost exposes `GET /v1/models/<alias>` which returns the resolved
OpenAI-style model object including the upstream `id`. If your Bifrost
version exposes only `GET /v1/models` (list), iterate.

### 4. Startup wiring

`SlotRouter::build_from_config()` (or whichever Phase 0 function does
startup-time slot resolution) gains:

```rust
let resolver = Arc::new(Resolver::new(bifrost_url, slot_aliases));
let main = resolver.resolve_slot(SlotName::Main).await
    .map_err(|e| RouterError::MainSlotUnavailable(e.into()))?;
let resolved_map = HashMap::from([(SlotName::Main, main)]);
for slot in [SlotName::Planner, SlotName::Classifier, SlotName::Verifier, ...] {
    match resolver.resolve_slot(slot).await {
        Ok(r) => { resolved_map.insert(slot, r); }
        Err(e) => {
            tracing::warn!(slot = ?slot, error = %e, "slot unresolvable; marking unavailable");
            // not inserted = effectively Unavailable
        }
    }
}
let router = SlotRouter { resolved: resolved_map, resolver };
```

`SlotRouter::resolve(slot)` returns `Option<&ResolvedSlot>` (Phase 0
returned a concrete value for main; keep that API for `resolve_main()`,
add `resolve_optional(slot)` for everything else).

### 5. Error variants

```rust
#[derive(thiserror::Error, Debug)]
pub enum RouterError {
    #[error("slot {0:?} not configured")]
    SlotNotConfigured(SlotName),
    #[error("slot {slot:?} alias {alias} not found at Bifrost (status {status})")]
    AliasNotFound { slot: SlotName, alias: String, status: reqwest::StatusCode },
    #[error("slot {slot:?} resolution: {source}")]
    Resolution { slot: SlotName, #[source] source: Box<dyn std::error::Error + Send + Sync> },
    #[error("main slot unavailable: {0}")]
    MainSlotUnavailable(Box<RouterError>),
}
```

### 6. DEBT update

`specs/phase-0/DEBT.md` #22: strike-through with date + commit ref.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core router::capability::
./scripts/spec-check.sh
```

`cargo run -p seasoned-hand-server` against a real Bifrost (or a wiremock
fixture) and observe log lines per slot: either `slot ... resolved
provider_model_id=...` or `slot ... unresolvable; marking unavailable`.

---

## Files changed

- `crates/seasoned-hand-core/src/router/capability/mod.rs` (new)
- `crates/seasoned-hand-core/src/router/capability/table.rs` (new)
- `crates/seasoned-hand-core/src/router/capability/tests.rs` (new)
- `crates/seasoned-hand-core/src/router/mod.rs` (modify — `pub mod capability;`,
  expose `Resolver`)
- `crates/seasoned-hand-core/src/router/slots.rs` (modify — wire `Resolver`
  into startup; `resolve_optional`)
- `crates/seasoned-hand-server/src/state.rs` (modify — pass Bifrost URL +
  alias map)
- `specs/phase-0/DEBT.md` (close #22)

---

## Spec references

- `/specs/phase-1/architecture.md` §4.4 (Bifrost interface), §6 (capability
  table fallback pay-down).
- `/specs/phase-0/stories/story-0.13.md` (Phase 0 capability auto-detect).
- `/specs/01-architecture/decisions/ADR-003-12-slot-model-routing.md`.

---

## Commit message

```
fix(phase-1): story 1.7 - Bifrost alias resolution + capability table (DEBT #22)

- router::capability::Resolver looks up each slot's alias via Bifrost
  GET /v1/models/<alias> at startup, recording the upstream provider
  model id; static capabilities_for(model_id) table covers Claude 4.x,
  GPT-5.x, and llama3.2:3b with a tri-state CapabilityFlags (Some(true)
  /Some(false)/None=unknown)
- main slot remains hard-required; non-main slots that fail to resolve
  log a warning and are recorded as unavailable (server still starts)
- SlotRouter::resolve_optional(slot) returns Option<&ResolvedSlot> for
  story 1.8's verifier ≠ main gate

Closes Phase 0 DEBT #22.

refs: /specs/phase-1/stories/story-1.7.md
```

---

## Notes for next story (1.8)

`Resolver::resolve_slot(SlotName::Main).provider_model_id` and
`...(SlotName::Verifier).provider_model_id` are now available at startup.
Story 1.8 compares them; equality fails the server.

For story 1.9 (Verifier Worker scaffolding), the resolver also surfaces
the verifier slot's *actual* provider model ID for prompts/logging.
