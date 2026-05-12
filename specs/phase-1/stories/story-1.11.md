# Story 1.11 — Invalidation Detector + Invalidation trigger

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 1.9b (Verifier Worker runtime — Invalidation
> trigger emits onto the `verify_request` stream this worker reads)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §2.4.2 trigger B
> (Invalidation), `/specs/phase-1/DEBT.md` #4 (single heuristic
> rationale), §8 (false-positive failure mode), `/specs/phase-1/stories/story-1.9.md`.

---

## Goal

Detect when a later observation contradicts an earlier assertion about file
content (the only heuristic Phase 1 ships) and enqueue a
`VerifyRequest::Invalidation` for the Verifier Worker. Hooks into the
existing PostToolUse hook chain; uses a per-session content-hash map; honors
an allow-list of "deliberate-rewrite" tools so legitimate edits don't fire.

## Acceptance criteria

- [ ] `seasoned-hand-core::verifier::invalidation::InvalidationDetector`
      PostToolUse hook holds, per session, a
      `HashMap<PathBuf, [u8; 32]>` mapping normalized paths to SHA-256 of
      the most-recent observed content.
- [ ] On every Observation event for a file-reading or file-writing tool:
      compute SHA-256 of the body, compare to the stored hash. **Trigger
      condition**: stored hash exists, differs from new hash, and the
      tool that produced the new observation is **not** on the allow-list
      `{"file_write", "file_str_replace"}`. After the comparison, update
      the stored hash to the new value regardless.
- [ ] Trigger emission: emit `Misc{kind:"verifier_request", data:
      {trigger:"Invalidation", reason: {kind:"FileMismatch", path,
      old_sha, new_sha}}}` and `XADD verify_request` with a
      `VerifyRequest::Invalidation { reason: FileMismatch{...} }`.
- [ ] VerifierGate (from story 1.10) on receipt of an `Invalidation`
      verdict: emit Misc `verifier_verdict` only; **no** session state
      transition (loop continues unchanged). On `fail` with
      `suggested_plan_update`, the Worker has already called
      `PlanManager::update` so the next agent iteration sees the new plan.
- [ ] Memory bound: detector evicts oldest entries when the per-session
      map exceeds 10,000 paths (configurable
      `verifier.invalidation.max_paths = 10000`). Heap stays well under
      the 1 MB-per-session budget (architecture §7).
- [ ] Allow-list is **closed**: only `file_write` and `file_str_replace`.
      Any other tool path (including shell-induced changes, browser-
      triggered downloads) triggers when content differs. Documented
      explicitly in code comments.
- [ ] Tests:
      - `hash_round_trip` — sha256 stable across runs.
      - `first_observation_does_not_trigger` — bookkeeping only.
      - `same_content_subsequent_observation_does_not_trigger`.
      - `different_content_via_allow_listed_tool_does_not_trigger` —
        observation via `file_write` after a different observation
        updates the hash but does **not** fire.
      - `different_content_via_shell_exec_triggers` — full integration:
        file_read returns `"v1"`, shell_exec then writes `"v2"`,
        file_read returns `"v2"` → assert Misc `verifier_request{
        trigger:"Invalidation"}` emitted and `XLEN verify_request == 1`.
      - `eviction_at_capacity` — populate 10,001 paths and assert the
        oldest is evicted.
      - `gate_does_not_transition_state_on_invalidation_verdict`.

## Non-goals

- Heuristics beyond file content hash mismatch — phase-1/DEBT.md #4
  explicitly tracks this for Phase 2/4.
- Detecting browser DOM-content drift, shell output contradiction, or
  plan-phase status conflict — all deferred.
- Per-session hash map cleanup on session end — handled by the existing
  Phase 0 SessionStore drop path (in-process map dies with the runtime).
- Web/browser content invalidation.

## Implementation steps

### 1. Module

```
crates/seasoned-hand-core/src/verifier/invalidation/
  mod.rs       — InvalidationDetector, InvalidationReason
  hook.rs      — PostToolUse hook integration
  tests.rs
```

### 2. Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InvalidationReason {
    FileMismatch { path: PathBuf, old_sha: String, new_sha: String },
}

pub struct InvalidationDetector {
    // per-session content-hash maps, evict oldest at capacity
    inner: DashMap<String /* session */, Mutex<LruCache<PathBuf, [u8; 32]>>>,
    capacity: usize,
}

impl InvalidationDetector {
    pub fn new(capacity: usize) -> Self { ... }
    pub async fn observe(
        &self,
        session_id: &str,
        tool_name: &str,
        path: PathBuf,
        body: &[u8],
    ) -> Option<InvalidationReason>;
}
```

`LruCache` from the `lru` crate (lightweight, single Cargo.toml addition
if not already present; otherwise use a hand-rolled `HashMap` +
`VecDeque` ringbuffer to track insertion order). Prefer reusing whatever
the workspace already vends.

### 3. Path extraction

A small helper in the hook turns a tool name + observation payload into
a normalized `PathBuf`:

| Tool | Source field |
|---|---|
| `file_read` | input arg `path` |
| `file_write` | input arg `path` |
| `file_str_replace` | input arg `path` |
| `file_find_in_content` | each line returns `path:line` — skip; not single-file |
| `shell_exec` | None — body opaque, no path; skip |

For Phase 1, only `file_read`/`file_write`/`file_str_replace`
observations carry single-file content. The detector's `observe()` is
only called when the path is known.

Edge case: `shell_exec` triggers an indirect file change. Detection
fires on the *next* `file_read` of the modified path — hash diff vs
stored hash, and the *tool that produced the new observation* is
`file_read`, which is not on the allow-list, so it triggers. (This is
the architecture's intended behavior — the agent's mental model of the
file was bypassed; the next read surfaces the drift.)

Wait — re-reading architecture §2.4.2 algorithm: the trigger condition
is "the tool path through which content changed". So actually, the
allow-list check should be on **the tool that produced the new
observation** (the read or write that surfaced the new hash), not on
the tool that *caused* the change. The architecture's example pseudo-
code treats `file_write` / `file_str_replace` as the deliberate edit
path; observing a new hash via `file_read` after an out-of-band
`shell_exec` triggers because `file_read` is not on the allow-list.
Our implementation matches that semantic.

### 4. Hook registration

Story 0.10's `EventEmittingHook` is the PostToolUse hook scaffold. Add
`InvalidationHook` as a sibling PostToolUse hook. Each tool's
`dispatch()` is wrapped; this hook runs *after* the tool returns and
the Observation has been emitted.

```rust
pub struct InvalidationHook { detector: Arc<InvalidationDetector>, redis: Arc<RedisPool> }

#[async_trait]
impl PostToolUseHook for InvalidationHook {
    async fn on_post_tool(&self, ctx: &HookContext, obs: &Observation) {
        let Some(path) = extract_path(&ctx.tool_name, &ctx.args, obs) else { return; };
        let body = match obs.body_bytes() {
            Some(b) => b,
            None => return,
        };
        if let Some(reason) = self.detector.observe(
            &ctx.session_id, &ctx.tool_name, path, &body
        ).await {
            let event_id = self.emit_request_misc(ctx, &reason).await;
            let req = VerifyRequest {
                session_id: ctx.session_id.clone(),
                trigger: VerifyTrigger::Invalidation { reason },
                triggered_at_event_id: event_id,
                context_hint: VerifyContextHint::default(),
            };
            self.redis.xadd_json("verify_request", &req).await.ok();
        }
    }
}
```

### 5. VerifierGate extension

```rust
(Some("Invalidation"), _) => {
    // No state transition. The loop continues. The Worker (story 1.9)
    // already applied any suggested_plan_update; the next agent
    // iteration sees the new plan via build_messages → PlanManager::snapshot.
    state.events.emit_misc(&ev.session_id, "verifier_handled_invalidation",
        json!({"verdict": ev.data.get("verdict")})).await.ok();
}
```

(The audit Misc is optional; the verdict event itself is already
preserved per PRINCIPLE #5.)

### 6. Configuration

`config/seasoned-hand.toml` (or wherever Phase 0 stores config) gains:

```toml
[verifier.invalidation]
max_paths = 10000
```

Default kept in code at `InvalidationDetector::DEFAULT_CAPACITY = 10000`.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core verifier::invalidation::
./scripts/spec-check.sh
```

Manual integration (sandbox required): create a session, `file_write
/workspace/foo.txt "v1"`, then `shell_exec "echo v2 > /workspace/foo.txt"`,
then `file_read /workspace/foo.txt`. Observe one
Misc `verifier_request{trigger:"Invalidation"}` event and (if the
verifier slot is live) a downstream `verifier_verdict` event.

---

## Files changed

- `crates/seasoned-hand-core/src/verifier/invalidation/mod.rs` (new)
- `crates/seasoned-hand-core/src/verifier/invalidation/hook.rs` (new)
- `crates/seasoned-hand-core/src/verifier/invalidation/tests.rs` (new)
- `crates/seasoned-hand-core/src/verifier/mod.rs` (modify — `pub mod
  invalidation;`, extend `VerifyTrigger`)
- `crates/seasoned-hand-core/src/dispatch/hooks.rs` (modify — register
  `InvalidationHook` as PostToolUse)
- `crates/seasoned-hand-core/src/verifier/gate.rs` (modify — handle
  Invalidation verdict arm)
- `Cargo.toml` (modify if adding `lru = "0.12"` and `sha2 = "0.10"`
  was not already brought in by story 1.9)
- `config/seasoned-hand.toml` (modify — `[verifier.invalidation]`)
- `crates/seasoned-hand-core/src/events/misc.rs` (modify — document
  `verifier_handled_invalidation`)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.4.2 trigger B (verbatim
  algorithm), §7 (heap budget), §8 (false-positive failure mode),
  §12 q8 (event-id stability).
- `/specs/phase-1/DEBT.md` #4 (single-heuristic rationale).
- `/specs/00-philosophy/PRINCIPLES.md` #5 (errors preserved).

---

## Commit message

```
feat(phase-1): story 1.11 - Invalidation Detector + Invalidation trigger

- verifier::invalidation::InvalidationDetector keeps per-session
  LRU(path → sha256[body]) up to max_paths=10000; PostToolUse hook
  hashes each file_read/file_write/file_str_replace observation,
  compares to the stored hash, fires when the new tool is NOT on the
  allow-list {file_write, file_str_replace}
- Trigger emits Misc verifier_request{trigger:"Invalidation",
  reason:FileMismatch{path, old_sha, new_sha}} + XADD verify_request
- VerifierGate handles the Invalidation verdict arm without
  transitioning session state; the Worker (story 1.9 handle_request)
  already applied any suggested_plan_update via PlanManager::update
  with source=Verifier
- Memory bounded; per-session heap stays under architecture §7 budget
- 7 tests cover hash stability, first-observation no-fire,
  allow-listed updates, out-of-band shell triggers, eviction at
  capacity, and gate non-transition

refs: /specs/phase-1/stories/story-1.11.md
```

---

## Notes for next story (1.12)

Two of three Verifier triggers are live (TaskComplete, Invalidation).
Story 1.12 wires the third — CircuitBreaker — by unifying the four
breaker conditions (Stuck, Cost, MaxSteps, ErrorRate) and routing
trips through the Verifier. Story 1.12 also lands the Diversity
Injector (PRINCIPLE #6) since it modifies the stuck-tracker code path
the breaker integrates with.
