# Story 0.12 — Model router (12-slot resolution + YAML config)

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: story 0.11 (LLM client)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/01-architecture/ARCHITECTURE.md` §3 (12-slot routing), `/specs/phase-0/architecture.md` §4.4, ADR-003

---

## Goal

Add the Rust-side 12-slot router that maps slot names (`main`,
`planner`, `verifier`, `vision`, `web_extract`, `screenshot`,
`compression`, `session_title`, `session_search`, `classifier`,
`embedding`, `reasoning`) to Bifrost-side model aliases via a YAML
config file. Architecture §3 calls this the "12-slot routing"
pattern; ADR-003 is the decision record.

Phase 0 scope: the resolver + YAML parser. Capability detection
(verify the model supports tool-calling) is story 0.13. Live use in
the agent loop is story 0.14.

## Acceptance criteria

- [ ] `seasoned-hand-core::router` module with:
      - `SlotName` enum: 12 variants (3 main + 9 auxiliary)
      - `SlotConfig` struct: `{ provider, model, base_url }` matching
        architecture §3.3 — `provider: auto | main | <name>`, `model:
        <id>`, `base_url: <override URL or None>`
      - `RouterConfig`: top-level YAML shape `{ slots: { main: SlotConfig, ... } }`
      - `SlotRouter::from_yaml(path) -> Result<Self, RouterError>`
      - `SlotRouter::from_yaml_str(s) -> Result<Self, RouterError>`
      - `SlotRouter::resolve(slot) -> ResolvedSlot { model, base_url, api_key? }`
- [ ] Special providers:
      - `provider: auto` falls back to the `main` slot's resolution
      - `provider: main` explicitly reuses the `main` slot (functionally
        equivalent to `auto` in Phase 0 but signals intent)
      - `base_url: <url>` overrides the provider-derived base URL (so a
        user can point any slot at any OpenAI-compatible endpoint)
- [ ] Reject startup if `main` slot is missing — that's a hard error
- [ ] `auxiliary` slots default to `auto` when unset (loading proceeds
      with a warning, not an error)
- [ ] `SlotRouter::resolve(SlotName::Main)` returns a resolved tuple even
      when the config is minimal (just `slots.main`)
- [ ] Example config at `config/slots.example.yaml` with all 12 slots
- [ ] Server `main.rs` loads `${SLOTS_CONFIG_PATH:-config/slots.yaml}` if
      present, otherwise builds a minimal default that points `main` at
      `agent-primary` (Bifrost alias from story 0.1)
- [ ] Unit tests cover: parse complete config, parse minimal config
      (main only), `auto` and `main` provider fall back to main, missing
      `main` errors at config-load time, base_url override wins,
      `from_yaml` reads a file
- [ ] `cargo clippy / fmt / test / spec-check` all pass

## Non-goals

- Capability detection (story 0.13)
- Calling the LLM via a slot (story 0.14)
- Live config reload (Phase 1)
- Per-slot rate limits (Phase 1)
- Per-slot cost caps (story 0.16 caps at the session level, not per slot)

---

## Implementation steps

### 1. Types

```rust
// crates/seasoned-hand-core/src/router/mod.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotName {
    Main, Planner, Verifier,
    Vision, WebExtract, Screenshot, Compression,
    SessionTitle, SessionSearch, Classifier, Embedding, Reasoning,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlotConfig {
    pub provider: String,            // free-form: openai | anthropic | ollama | main | auto | ...
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>, // e.g. "ANTHROPIC_API_KEY"; resolved at startup
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouterConfig {
    pub slots: std::collections::HashMap<SlotName, SlotConfig>,
}

#[derive(Debug, Clone)]
pub struct ResolvedSlot {
    pub provider: String,
    pub model: String,            // either user-specified or inherited from main
    pub base_url: Option<String>, // override URL
    pub api_key: Option<String>,
}

pub struct SlotRouter { /* resolved map */ }
```

### 2. Resolution logic

`SlotRouter::from_config(cfg)`:
- Validate `slots.main` exists; error otherwise
- For each slot, build a `ResolvedSlot`:
  - If `provider == "auto"` or `provider == "main"` → inherit from main's
    resolved slot
  - Else use the slot's own model + base_url, with `api_key` resolved
    from `api_key_env` (read env var; absent → None)
  - `base_url` from the slot wins; falls back to main's `base_url`
    (Bifrost) if not set
- Store in `HashMap<SlotName, ResolvedSlot>`; `resolve(slot)` returns a
  ref

### 3. YAML config example

`config/slots.example.yaml`:

```yaml
slots:
  main:
    provider: anthropic
    model: claude-sonnet-4-6
    base_url: http://localhost:4000/v1   # via Bifrost
    api_key_env: ANTHROPIC_API_KEY

  planner:
    provider: auto

  verifier:
    provider: openai
    model: gpt-4o
    base_url: http://localhost:4000/v1

  vision:
    provider: auto
  web_extract:
    provider: auto
  screenshot:
    provider: auto
  compression:
    provider: auto
  session_title:
    provider: auto
  session_search:
    provider: auto
  classifier:
    provider: auto
  embedding:
    provider: openai
    model: text-embedding-3-small
    base_url: http://localhost:4000/v1
  reasoning:
    provider: auto
```

### 4. Server wiring

`main.rs`:

```rust
let slots_path = std::env::var("SLOTS_CONFIG_PATH")
    .unwrap_or_else(|_| "config/slots.yaml".into());
let router = if std::path::Path::new(&slots_path).exists() {
    SlotRouter::from_yaml(&slots_path)?
} else {
    tracing::warn!(%slots_path, "slots config not found, using built-in default (main → agent-primary)");
    SlotRouter::default_for_bifrost()
};
```

`SlotRouter::default_for_bifrost()` builds a minimal router with only
`main` set to `agent-primary`, base_url Bifrost. Used when no config
file is present (Phase 0 dev ergonomics).

### 5. AppState / dispatcher integration

`AppState` gains `router: Arc<SlotRouter>` (Phase 0 wires it but doesn't
yet call it from the agent loop — that's story 0.14). For now, just
expose via state; tests assert the resolver works without a live loop.

### 6. Tests

- `parse_full_yaml_config` — 12 slots, all fields
- `parse_minimal_yaml_only_main` — passes
- `parse_missing_main_errors` — error at config-load
- `auto_inherits_from_main`
- `main_provider_inherits_from_main` (alias of auto)
- `base_url_override_wins` — slot's base_url ≠ main's
- `api_key_env_resolves_from_env` — set/unset cases
- `from_yaml_reads_file` — round-trip via tempfile

---

## Files changed

- `crates/seasoned-hand-core/Cargo.toml` (`serde_yaml = "0.9"` if not already)
- `crates/seasoned-hand-core/src/lib.rs` (`pub mod router`)
- `crates/seasoned-hand-core/src/router/mod.rs` (new)
- `crates/seasoned-hand-core/src/router/tests.rs` (new)
- `config/slots.example.yaml` (new)
- `.gitignore` (add `config/slots.yaml` — local-only) — verify, may
  already be ignored as `**/*.local.*` pattern
- `crates/seasoned-hand-server/src/lib.rs` (`AppState.router` field)
- `crates/seasoned-hand-server/src/main.rs` (load slots config)
- `crates/seasoned-hand-server/tests/healthz.rs` + `events.rs` (update
  AppState construction)

---

## Spec references

- `/specs/01-architecture/ARCHITECTURE.md` §3 (12-slot routing)
- `/specs/01-architecture/decisions/ADR-003-12-slot-model-routing.md`
- `/specs/phase-0/architecture.md` §4.4

---

## Commit message

```
feat(phase-0): story 0.12 - 12-slot model router + YAML config

- seasoned-hand-core::router with SlotName (12 variants), SlotConfig,
  RouterConfig, ResolvedSlot, SlotRouter
- YAML config parsed via serde_yaml; main slot required, others
  default to provider:"auto" inheriting from main
- Special providers: "auto" and "main" both inherit; "base_url"
  override wins
- api_key resolution via api_key_env field
- SlotRouter::default_for_bifrost() minimal default when no config
  file present (main → agent-primary, base_url Bifrost)
- AppState gains router field (Phase 0 just wires it; agent loop uses
  it in story 0.14)
- config/slots.example.yaml shipped with all 12 slots documented
- N unit tests cover: full+minimal parse, auto/main inheritance,
  base_url override, api_key_env, missing main rejected
- cargo clippy / fmt / test / spec-check all pass

refs: /specs/phase-0/stories/story-0.12.md
```
