# Story 1.5 — Tool-mask layer (PRINCIPLE #2)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 1.4 (Initializer exists; `plan_create` is the immediate
> mask candidate)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §2.3 (PRINCIPLE #2
> enforcement table), §4.3 (tool catalog), `/specs/00-philosophy/PRINCIPLES.md`
> #2 (one tool per iteration; stable catalog).

---

## Goal

Implement the **tool-mask layer**: a per-iteration filter that marks a tool
as `available:false` in the schema **description** but keeps the tool in the
catalog. The KV-cache stable prefix is preserved (PRINCIPLE #2: no
mid-iteration catalog reorder/remove). First consumer: hide `plan_create`
and `checkpoint_rollback` from the LLM at every iteration; story 1.13 will
add `checkpoint_rollback` to the registry as internal-only — this story
ships the mechanism so 1.13 just supplies a mask.

## Acceptance criteria

- [ ] `seasoned-hand-core::dispatch::mask::ToolMask` trait extension over
      `Tool`:
      ```rust
      pub trait ToolMaskPolicy: Send + Sync {
          fn is_available(&self, tool_name: &str, ctx: &MaskContext) -> bool;
      }
      ```
- [ ] `MaskContext { session_id: SessionId, iteration: u32, mode: AgentMode }`
      where `AgentMode { Initializer | Worker | Verifier }` (Verifier exists
      conceptually before story 1.9 ships it; the enum variant is added
      here).
- [ ] When building the LLM tool list, unavailable tools are **not**
      removed; their `description` is wrapped in
      `"[UNAVAILABLE in current iteration] " + original_description` and
      their JSON Schema is unchanged.
- [ ] The dispatcher rejects any LLM call to an unavailable tool with
      `ToolOutput { ok:false, error:"tool_unavailable_in_iteration",
      details:{tool, mode}}` and emits Misc `tool_mask_violation`.
- [ ] Built-in mask policy `DefaultMaskPolicy`:
      - `plan_create` → unavailable in `Worker` (Initializer-only).
      - `checkpoint_rollback` → unavailable in all LLM-facing modes (this
        tool is always backend-driven).
- [ ] Tool catalog **order** is byte-stable between iterations (property
      test against a 10-iteration synthetic run).
- [ ] Tests:
      - `mask_descriptions_prefix_unavailable` — generate the tool list
        for a Worker iteration; assert `plan_create.description` starts
        with `"[UNAVAILABLE in current iteration] "`.
      - `mask_does_not_change_order` — produce tool list at iterations
        0, 1, 50; assert ordered list of `(name, schema_hash)` is byte-
        identical.
      - `dispatcher_rejects_masked_tool` — wiremock LLM emits a
        `plan_create` tool_call from a Worker iteration; assert
        ToolOutput.ok=false + `tool_mask_violation` Misc event emitted.
      - `initializer_can_still_call_plan_create_directly` — Initializer
        constructs `MaskContext { mode: Initializer }`; assert dispatch
        succeeds for the same call.
      - `spec_check_asserts_no_reorder` — extend `scripts/spec-check.sh`
        with a Rust unit test reference that fails CI if the catalog
        order is altered (test exists in this story; CI green confirms).

## Non-goals

- Per-tool masking based on runtime context other than `AgentMode` —
  Phase 4 Curator may add task-specific masking; out of Phase 1.
- Hiding tools by removing them from the catalog (explicit anti-goal:
  this violates PRINCIPLE #2).
- The `checkpoint_rollback` tool registration itself — story 1.13. This
  story only ships the mask entry; the registration in 1.13 produces a
  matching catalog entry.

## Implementation steps

### 1. Module

```
crates/seasoned-hand-core/src/dispatch/mask.rs
```

```rust
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub enum AgentMode { Initializer, Worker, Verifier }

#[derive(Debug, Clone)]
pub struct MaskContext {
    pub session_id: String,
    pub iteration: u32,
    pub mode: AgentMode,
}

pub trait ToolMaskPolicy: Send + Sync {
    fn is_available(&self, tool_name: &str, ctx: &MaskContext) -> bool;
}

pub struct DefaultMaskPolicy;

impl ToolMaskPolicy for DefaultMaskPolicy {
    fn is_available(&self, name: &str, ctx: &MaskContext) -> bool {
        use AgentMode::*;
        match (name, ctx.mode) {
            ("plan_create",          Worker)       => false,
            ("plan_create",          Verifier)     => false,
            ("checkpoint_rollback",  _)            => false,
            _                                       => true,
        }
    }
}

pub fn apply_mask(
    specs: &mut [ToolSpec],
    policy: &dyn ToolMaskPolicy,
    ctx: &MaskContext,
) {
    for s in specs.iter_mut() {
        if !policy.is_available(&s.function.name, ctx) {
            s.function.description = Some(format!(
                "[UNAVAILABLE in current iteration] {}",
                s.function.description.as_deref().unwrap_or("")
            ));
        }
    }
}
```

### 2. Wire into AgentRunner

In `AgentRunner::run`, before constructing the LLM call:

```rust
let mut specs = self.tool_specs_from_registry();
let mask_ctx = MaskContext { session_id: req.session_id.clone(), iteration: step, mode: AgentMode::Worker };
apply_mask(&mut specs, &*self.mask_policy, &mask_ctx);
```

`AgentRunner` holds `mask_policy: Arc<dyn ToolMaskPolicy>` constructed in
`AppState::new` with `Arc::new(DefaultMaskPolicy)`. Configurable via
config later (out of Phase 1).

### 3. Dispatcher enforcement

In `ToolDispatcher::dispatch`, immediately after lookup:

```rust
if !self.mask_policy.is_available(&name, &ctx.mask_ctx) {
    let out = ToolOutput::err("tool_unavailable_in_iteration",
        json!({"tool": name, "mode": format!("{:?}", ctx.mask_ctx.mode)}));
    ctx.events.emit_misc(&ctx.session_id, "tool_mask_violation",
        json!({"tool": name, "mode": format!("{:?}", ctx.mask_ctx.mode)})).await?;
    return out;
}
```

The Initializer's `MaskContext::mode = Initializer` so its programmatic
`plan_create` calls pass through.

### 4. Catalog order stability

Add a unit test in `crates/seasoned-hand-core/src/tools/registry/tests.rs`:

```rust
#[test]
fn tool_catalog_order_is_stable() {
    let r = ToolRegistry::default();
    let names0: Vec<_> = r.specs().iter().map(|s| s.function.name.clone()).collect();
    for _ in 0..10 {
        let names_n: Vec<_> = r.specs().iter().map(|s| s.function.name.clone()).collect();
        assert_eq!(names0, names_n);
    }
}
```

Plus a property test using `proptest` that simulates `apply_mask` with
random mask outputs and asserts the name order is unchanged.

### 5. spec-check assertion

Append to `scripts/spec-check.sh` a check that searches for the literal
test name `tool_catalog_order_is_stable` in the workspace — fails if
removed:

```bash
grep -q 'fn tool_catalog_order_is_stable' crates/seasoned-hand-core/src/tools/registry/tests.rs \
    || { echo "ERROR: tool_catalog_order_is_stable test missing — see story 1.5"; exit 1; }
```

(This is a *meta*-check that the test exists, not a re-run of the test.)

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core dispatch::mask::
cargo test -p seasoned-hand-core tools::registry::tests::tool_catalog_order_is_stable
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/dispatch/mask.rs` (new)
- `crates/seasoned-hand-core/src/dispatch/mod.rs` (modify — `pub mod mask;`)
- `crates/seasoned-hand-core/src/dispatch/context.rs` (modify — add
  `mask_ctx: MaskContext`)
- `crates/seasoned-hand-core/src/dispatch/dispatcher.rs` (modify — mask
  enforcement before dispatch)
- `crates/seasoned-hand-core/src/agent/mod.rs` (modify — `mask_policy`
  field, apply before LLM call)
- `crates/seasoned-hand-core/src/agent/init/mod.rs` (modify — pass
  `AgentMode::Initializer` when calling `plan_create`)
- `crates/seasoned-hand-core/src/tools/registry/tests.rs` (new file or
  modify existing — add stability test)
- `crates/seasoned-hand-server/src/state.rs` (modify — build
  `mask_policy: Arc<dyn ToolMaskPolicy>`)
- `scripts/spec-check.sh` (modify — meta-check for stability test name)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.3 (table row #2), §4.3 (catalog).
- `/specs/00-philosophy/PRINCIPLES.md` #2.

---

## Commit message

```
feat(phase-1): story 1.5 - tool-mask layer (PRINCIPLE #2)

- dispatch::mask::{ToolMaskPolicy, MaskContext, AgentMode,
  DefaultMaskPolicy} masks tools per iteration without removing them
  from the catalog; description is prefixed with "[UNAVAILABLE in
  current iteration] "; schema is unchanged
- Dispatcher rejects masked calls with tool_unavailable_in_iteration +
  Misc tool_mask_violation event
- DefaultMaskPolicy hides plan_create from Worker/Verifier modes
  (Initializer can still call it programmatically) and
  checkpoint_rollback from all LLM modes (always backend-driven)
- Tool catalog name-order is byte-stable across iterations
  (proptest + 10-iter unit test); spec-check.sh meta-asserts the test
  exists
- 5 unit tests; agent runner now applies the mask before every LLM call

refs: /specs/phase-1/stories/story-1.5.md
```

---

## Notes for next story (1.6)

Catalog stability + masking is in place. Story 1.13 (Checkpoint Manager)
registers `checkpoint_rollback` and gets free masking via
`DefaultMaskPolicy`. Story 1.6 (Context Recitation) is independent and
can be worked in parallel.

Future-Verifier flow: when story 1.9 wires the Verifier Worker, its
internal `plan_update` invocation runs in `AgentMode::Verifier` and the
mask policy already permits it.

---

## Execution notes (post-Phase-1 simplicity pass)

**Spec divergence — `MaskContext` collapsed to bare `AgentMode`.** The
story sketched `MaskContext { session_id, iteration, mode }` and a
trait method `is_available(&self, tool_name: &str, ctx: &MaskContext)`.
The post-Phase-1 simplicity pass (commit `1aacf18`) dropped
`MaskContext` entirely:
- `DefaultMaskPolicy` only ever read `ctx.mode`; `session_id` and
  `iteration` were unread in production.
- The `Iteration`-conditional `ToggleMaskPolicy` test impl was
  tautologically proving "`iter_mut().for_each()` doesn't reorder" —
  a property of the loop, not of the policy.
- `ToolContext.mask_ctx: MaskContext` became `ToolContext.mask_mode:
  AgentMode`. Trait method is now
  `is_available(&self, tool_name: &str, mode: AgentMode)`.

When a future story needs per-session or per-iteration masking, the
trait surface can be widened then. Phase 1 does not need it.

**Spec divergence — `mask_does_not_change_order` test removed.** That
test exercised `apply_mask` with the now-deleted `ToggleMaskPolicy` to
prove `iter_mut` order stability across iterations. Since iter_mut
cannot reorder by construction, the test was redundant; removal at
`1aacf18`. The shipped `tool_catalog_order_is_stable` test asserts
catalog name-order stability over the `sample_specs()` helper, which
is a weaker but spec-check-meta-asserted property (`scripts/spec-check.sh`
greps for the test by name).
