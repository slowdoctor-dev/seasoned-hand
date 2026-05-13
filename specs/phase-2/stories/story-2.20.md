# Story 2.20 — NarratorHook classifier-slot wiring through AppState::new

> **Status**: ready
> **Estimated**: 1.5 hours
> **Dependencies**: —
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-1/stories/story-1.15.md` "Execution notes"

---

## Goal

Close out the deferred plumbing from Phase 1 story 1.15: today
`NarratorHook` is registered at `AppState::new` with a templated-only
configuration — action-changing tools (`file_write`, `shell_*`,
`browser_*`) fall through to the generic `"Invoking {tool}"` because
the classifier-slot is never wired. This story wires it.

## Acceptance criteria

- [ ] `AppState::new` accepts an `Option<NarratorClassifierWiring>`
      parameter (additive — main.rs passes `Some(...)` when the
      narrator classifier prompt is loaded; tests pass `None`):
      ```rust
      pub struct NarratorClassifierWiring {
          pub llm: Arc<LlmClient>,
          pub model: String,        // classifier slot alias
          pub system_prompt: Arc<String>,
      }
      ```
- [ ] When `Some(...)`, the `NarratorHook` registered into the
      dispatcher is built with
      `NarratorHook::new(events).with_classifier(llm, model, prompt)`.
      When `None`, the templated-only construction stays.
- [ ] `main.rs` loads the system prompt from
      `config/prompts/narrator.system.txt` (the file already exists
      from story 1.15). If the file is missing, log a warning and
      proceed templated-only (graceful degradation).
- [ ] `main.rs` resolves the classifier slot via
      `state.router.resolve(SlotName::Classifier)`; constructs a
      `LlmClient` against the slot's base_url + api_key.
- [ ] `main.rs` constructs the wiring AFTER `AppState::new` finishes —
      wait, that contradicts the additive-arg approach. Better: add
      a builder method on `AppState`:
      ```rust
      pub fn with_narrator_classifier(self, wiring: NarratorClassifierWiring) -> Self;
      ```
      Builder pattern matches `with_verifier_prompt` / `with_admin_token`
      / `with_rollback_on_verifier_fail` from Phase 1. Internal: rebuild
      the dispatcher with a new NarratorHook (carrying the classifier).
- [ ] Tests:
      - `with_narrator_classifier_attaches_classifier`
      - `main_rs_loads_classifier_prompt_when_present`
      - `main_rs_degrades_to_templated_when_prompt_missing`
      - Existing `llm_path_calls_classifier_slot` test from story 1.15
        already covers the LLM-call surface — re-verify it still passes
        unchanged.

## Non-goals

- Changing `NarratorHook` internals — the `with_classifier` builder
  already exists in story 1.15.
- Per-tenant classifier configuration (Phase 5).

---

## Implementation steps

### 1. AppState builder method

`crates/seasoned-hand-server/src/lib.rs`:

```rust
impl AppState {
    pub fn with_narrator_classifier(mut self, wiring: NarratorClassifierWiring) -> Self {
        // Rebuild the dispatcher with a NarratorHook that carries the classifier.
        // Same pattern as the verifier_prompt builder.
        let narrator = Arc::new(
            NarratorHook::new(self.events.clone())
                .with_classifier(wiring.llm, wiring.model, wiring.system_prompt)
        );
        let dispatcher = ToolDispatcher::new(register_builtin_tools())
            .with_hook(narrator)
            .with_hook(Arc::new(EventEmittingHook::new(self.events.clone())))
            .with_hook(Arc::new(InvalidationHook::new(
                self.events.clone(),
                Some(self.redis.clone()),
            )))
            .with_hook(Arc::new(PostBrowserActionHook::new(self.events.clone())));
        self.dispatcher = Arc::new(dispatcher);
        self
    }
}
```

### 2. main.rs

```rust
let prompt_path = std::env::var("NARRATOR_PROMPT_PATH")
    .unwrap_or_else(|_| "config/prompts/narrator.system.txt".to_string());
match std::fs::read_to_string(&prompt_path) {
    Ok(prompt) => {
        let classifier_slot = state.router.resolve(SlotName::Classifier);
        let llm = LlmClient::new(
            classifier_slot.base_url.clone(),
            classifier_slot.api_key.clone(),
        );
        state = state.with_narrator_classifier(NarratorClassifierWiring {
            llm: Arc::new(llm),
            model: classifier_slot.model.clone(),
            system_prompt: Arc::new(prompt),
        });
        tracing::info!(path = %prompt_path, "narrator classifier wired");
    }
    Err(error) => {
        tracing::warn!(
            %error, path = %prompt_path,
            "narrator classifier prompt missing; narration falls through to templated-only"
        );
    }
}
```

### 3. Tests

Build an AppState in test mode without `with_narrator_classifier`,
assert the dispatcher's NarratorHook uses templated-only (probe via
emitted Message kind / content). Build a second AppState WITH the
classifier wired, assert action-changing tools route through the LLM
path. Use wiremock for the LLM endpoint.

### 4. story-1.15 exec note update

Add a small note to `specs/phase-1/stories/story-1.15.md` Execution
notes referencing this story (2.20) as the closing of the deferred
plumbing.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-server narrator
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-server/src/lib.rs` (modify — new builder
  method + dispatcher rebuild)
- `crates/seasoned-hand-server/src/main.rs` (modify — load prompt +
  apply builder)
- `crates/seasoned-hand-server/tests/narrator_wiring.rs` (new)
- `specs/phase-1/stories/story-1.15.md` (modify — exec-note close)

---

## Spec references

- `/specs/phase-1/stories/story-1.15.md` Execution notes ("Deferred —
  classifier-slot wiring through AppState::new")

---

## Commit message

```
feat(phase-2): story 2.20 - NarratorHook classifier-slot wiring through AppState::new

Closes the deferred plumbing called out in story 1.15 Execution
notes. Before this commit, NarratorHook was templated-only at boot —
action-changing tools (file_write, shell_*, browser_*) fell through
to "Invoking {tool}" because the classifier slot was never wired.

- AppState gains with_narrator_classifier(wiring) builder. Rebuilds
  the dispatcher with a NarratorHook carrying the classifier.
  Matches the with_verifier_prompt / with_admin_token /
  with_rollback_on_verifier_fail Phase 1 pattern.
- main.rs loads config/prompts/narrator.system.txt at boot; on
  missing file, logs a warning and degrades to templated-only.
- LLM client built against router.resolve(SlotName::Classifier).
- 3 new tests + existing story-1.15 LLM-path test stays green.

refs: /specs/phase-2/stories/story-2.20.md
refs: /specs/phase-1/stories/story-1.15.md (exec-note close)
```

---

## Notes for next story (2.21)

All three DEBT close-outs done (#14 + #15 + 1.15-wiring). 2.21 ships
the `seasoned-hand` CLI binary — the "OS layer" final piece.
