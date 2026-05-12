# Story 1.8 — Verifier slot startup gate (verifier ≠ main resolved-model-ID)

> **Status**: ready
> **Estimated**: 1 hour
> **Dependencies**: 1.7 (capability resolver provides
> `ResolvedSlot::provider_model_id`)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §2.4.3 (Startup
> check), §4.4 (Bifrost interface), `/specs/01-architecture/ARCHITECTURE.md`
> §6 L4 (verifier different model).

---

## Goal

Hard-fail server startup when the `verifier` slot resolves to the same
provider model ID as `main`. Prevents the entire L4 meta-cognition
capability from silently collapsing into self-consistency bias due to
misconfiguration. Lands **before** the Verifier Worker scaffolding (story
1.9) so a bad config surfaces *before* any verifier code path is exercised.

## Acceptance criteria

- [ ] `SlotRouter::build_from_config()` (or whichever startup function
      assembles slot resolution) runs the gate after both `main` and
      `verifier` resolve. If `main.provider_model_id ==
      verifier.provider_model_id`, return a startup error that names
      both: `"verifier slot must use a different model than main; both
      resolved to: <id>"`.
- [ ] The check runs *only* if the `verifier` slot is configured at all.
      A workspace with no verifier slot (Phase 0 backwards compatibility
      / hosted-by-config-omission) emits a `tracing::info!("verifier slot
      not configured — L4 meta-cognition disabled")` and starts normally.
- [ ] Verifier-disabled mode is **recorded**: `AppState::verifier_enabled:
      bool` defaults false; flips true only when the gate passes with a
      configured slot.
- [ ] Comparison is at the resolved provider model ID level, **not** the
      alias level. (Two different aliases pointing at the same upstream
      model both fail.)
- [ ] Error variant: `RouterError::VerifierSameAsMain { model_id: String }`.
- [ ] Tests:
      - `gate_passes_when_models_differ` — main=`claude-sonnet-4-6`,
        verifier=`gpt-5.1`; build succeeds; `verifier_enabled = true`.
      - `gate_fails_when_models_equal` — same `claude-sonnet-4-6` on
        both; build returns `VerifierSameAsMain`.
      - `gate_fails_when_aliases_differ_but_models_equal` — Bifrost
        returns the same `provider_model_id` for two distinct aliases
        (`agent-primary` and `agent-fallback` both pointing at
        `claude-sonnet-4-6`); build still fails.
      - `gate_skipped_when_verifier_not_configured` — slot alias map
        omits `Verifier`; build succeeds; `verifier_enabled = false`;
        info log line present.
      - `error_message_names_both_models` — `VerifierSameAsMain.to_string()`
        contains the offending model id.

## Non-goals

- Validating other slot pairs (Phase 4+ may compare verifier and planner
  for diversity reasons; out of Phase 1).
- Probing whether verifier and main are from the *same vendor* but
  different model — only ID equality matters in Phase 1.
- Surfacing the gate result via a health endpoint — log + startup error
  are sufficient.
- Reading `/config/prompts/verifier.system.txt` — that's story 1.9.

## Implementation steps

### 1. Error variant

In `crates/seasoned-hand-core/src/router/error.rs` (or wherever
`RouterError` lives):

```rust
#[error("verifier slot must use a different model than main; both resolved to: {model_id}")]
VerifierSameAsMain { model_id: String },
```

### 2. Gate

In `SlotRouter::build_from_config()` after main + verifier resolve:

```rust
let main = router.resolve_main();
let verifier_enabled = match router.resolve_optional(SlotName::Verifier) {
    Some(v) => {
        if v.provider_model_id == main.provider_model_id {
            return Err(RouterError::VerifierSameAsMain {
                model_id: v.provider_model_id.clone(),
            });
        }
        tracing::info!(
            main_model = %main.provider_model_id,
            verifier_model = %v.provider_model_id,
            "verifier slot validated; L4 meta-cognition enabled"
        );
        true
    }
    None => {
        tracing::info!("verifier slot not configured — L4 meta-cognition disabled");
        false
    }
};
router.verifier_enabled = verifier_enabled;
```

### 3. AppState plumbing

`AppState::verifier_enabled: bool` set from `router.verifier_enabled`.
Story 1.9 (Verifier Worker) reads this at boot — if false, the worker
task is not spawned.

### 4. Documentation in CHANGELOG.md

Append under `[Unreleased]` (or the working section): "Server now hard-
fails on startup when the verifier and main slots resolve to the same
provider model. See story 1.8 / architecture.md §2.4.3."

### 5. Tests

Pure-unit tests over a `SlotRouter::build_from_config` against a
wiremock'd Bifrost or a synthetic `Resolver` test double. Resolver
behavior is mocked by injecting precomputed `ResolvedSlot` values rather
than going through the real HTTP path.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core router::slots::verifier_gate
./scripts/spec-check.sh
```

Manual: configure `slots.yaml` with verifier=main alias; `cargo run -p
seasoned-hand-server` should exit with the documented error message and
non-zero status.

---

## Files changed

- `crates/seasoned-hand-core/src/router/error.rs` (modify — variant)
- `crates/seasoned-hand-core/src/router/slots.rs` (modify — gate)
- `crates/seasoned-hand-core/src/router/tests.rs` (modify — 5 unit tests)
- `crates/seasoned-hand-server/src/state.rs` (modify — propagate
  `verifier_enabled`)
- `CHANGELOG.md` (modify — one-line entry under Unreleased)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.4.3 (startup check rationale —
  self-consistency bias from Manus validation Q&A), §4.4 (defaults table).
- `/specs/01-architecture/ARCHITECTURE.md` §6 L4 (different model
  required for L4 meta-cognition).

---

## Commit message

```
feat(phase-1): story 1.8 - verifier ≠ main startup gate

- SlotRouter::build_from_config hard-fails with
  RouterError::VerifierSameAsMain { model_id } when verifier and main
  slots resolve to the same provider model id (alias-equality is NOT
  enough — comparison is on resolved provider_model_id, so two aliases
  pointing at the same upstream model also fail)
- Verifier slot absent → info log + verifier_enabled=false (server
  starts; story 1.9 worker is then not spawned)
- AppState::verifier_enabled exposes the result for downstream wiring
- 5 unit tests cover differ / equal / alias-but-same / not-configured /
  error-message-contents

refs: /specs/phase-1/stories/story-1.8.md
```

---

## Notes for next story (1.9)

`AppState::verifier_enabled` is the boot signal for story 1.9: only spawn
the Verifier Worker task if true. Story 1.9 also reads
`SlotRouter::resolve_optional(SlotName::Verifier)` to know which slot to
call for every verification run.
