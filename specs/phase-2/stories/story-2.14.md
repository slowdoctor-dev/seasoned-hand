# Story 2.14 — task_deliver LLM tool + RendererDispatcher wiring

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 2.6, 2.3
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.3 (deliverable standards), §8 "Renderer rendering failure"

---

## Goal

The LLM tool that lets the Worker hand back a real-employee artifact.
Wires `RendererDispatcher` (story 2.6) into a new dispatcher tool +
adds the "simplify and retry" fallback path the architecture's §8
calls for.

## Acceptance criteria

- [ ] New LLM tool `task_deliver` registered into the builtin tool
      catalog (Phase 0 0.7). Tool catalog count moves from 37 → 38.
      `scripts/spec-check.sh` expected count updates to 38.
- [ ] Tool schema:
      ```json
      {
        "type": "object",
        "properties": {
          "content": { "type": "string" },
          "target_filename": { "type": "string" },
          "citations": { "type": "array", "items": { "type": "integer" } }
        },
        "required": ["content", "target_filename"]
      }
      ```
- [ ] Mask policy: `task_deliver` is **Worker-mode only**. Initializer
      and Verifier modes get the tool masked (story 1.5 pattern;
      `DefaultMaskPolicy::is_available("task_deliver",
      AgentMode::Worker)` → `true`; all others → `false`).
- [ ] Handler flow:
      1. Validate `target_filename` extension via
         `DeliverableFormat::from_filename` (story 2.7). Unknown
         extension → `ToolError::InvalidArgs("unknown_format")`.
      2. Persist source content to
         `/workspace/.deliverables/.source/<deliverable_id>.<src_ext>`
         via SandboxClient::write_workspace_file.
      3. Call `RendererDispatcher::render(content, target_filename,
         &sandbox, session_id)`.
      4. On render success: persist `Deliverable` row via
         `DeliverableStore::insert`. `citations` stored as JSON array.
         `provenance_manifest` is the empty stub `{schema_version: 1,
         "task_id": ..., ...}` — story 2.15 wires the full manifest
         builder.
      5. Emit `Misc{kind: "deliverable", deliverable_id, format,
         file_ref}` event so the WS surfaces it.
      6. Return tool output `{ok: true, output: {deliverable_id,
         filename, format, content_sha256, content_size}}`.
- [ ] On renderer failure (`RenderError::RendererFailed`): **one
      retry** via "simplify content" LLM call:
      - Build a small prompt: "The renderer for {format} failed with
        stderr: {stderr_preview}. Simplify the content (remove
        complex tables / images / fancy formatting) while preserving
        the meaning. Return ONLY the new content."
      - Use the planner slot (since it's not a tool-calling step).
      - Re-attempt render with the simplified content.
      - If second attempt fails: fall back to writing the source as
        `.md` (raw), emit `Misc{kind:"deliverable_format_fallback",
        target_format: format, fell_back_to: "md", reason}`, persist
        Deliverable with `format = "md"`.
- [ ] Unit tests:
      - `task_deliver_writes_source_and_renders` (wiremock'd
        renderer + DB roundtrip)
      - `task_deliver_rejects_unknown_extension`
      - `task_deliver_emits_misc_deliverable_event`
      - `task_deliver_masked_in_initializer_mode`
      - `task_deliver_masked_in_verifier_mode`
      - `task_deliver_retries_with_simplified_content_on_render_fail`
      - `task_deliver_falls_back_to_md_after_double_fail`

## Non-goals

- Provenance manifest building (story 2.15 — this story persists the
  schema-version-only stub).
- DeliveryRouter dispatch to the channel (story 2.5 already does
  this; story 2.14 just emits the `Misc{kind:"deliverable"}` event
  that triggers it).

---

## Implementation steps

### 1. Tool in builtin catalog

`crates/seasoned-hand-core/src/tools/builtin.rs` — add `TaskDeliver`
struct + `impl Tool`. Same pattern as Phase 1 `CheckpointLabel` (story
1.13). Registered into `register_builtin_tools()`.

### 2. Mask policy

`crates/seasoned-hand-core/src/dispatch/mask.rs` `DefaultMaskPolicy`:
add the `("task_deliver", AgentMode::Worker) => true` arm and
`("task_deliver", _) => false`.

### 3. Tool handler

```
crates/seasoned-hand-core/src/deliverable/task_deliver.rs
```

Or inline in `builtin.rs` — match the Phase 1 convention.

### 4. Simplify-and-retry helper

```rust
async fn simplify_via_llm(
    llm: &LlmClient,
    planner_slot: &ResolvedSlot,
    failed_content: &str,
    stderr_preview: &str,
    target_format: DeliverableFormat,
) -> Result<String, LlmError> { ... }
```

Borrows the planner slot from `ToolContext::router` (since the slot
router is already in the context per Phase 1 1.7).

### 5. spec-check update

Bump expected tool count to 38 in `scripts/spec-check.sh`.

### 6. Tests

Inline-test the simplify-and-retry by wiremocking two LLM responses
(simplification + acknowledgement of the second render). The double-
fail fallback test asserts the `format` column in the persisted
Deliverable row is `"md"`.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core deliverable::task_deliver
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/tools/builtin.rs` (modify — TaskDeliver
  + registration)
- `crates/seasoned-hand-core/src/dispatch/mask.rs` (modify — mask arm)
- `crates/seasoned-hand-core/src/deliverable/task_deliver.rs` (new —
  the handler + simplify helper)
- `crates/seasoned-hand-core/src/tools/tests.rs` (modify — add
  task_deliver to the EXPECTED_TOOLS list)
- `scripts/spec-check.sh` (modify — bump expected count 37 → 38)

---

## Spec references

- `/specs/phase-2/architecture.md` §2.3 (deliverable formats), §8
  (render failure recovery), §11 (test list)

---

## Commit message

```
feat(phase-2): story 2.14 - task_deliver LLM tool + RendererDispatcher wiring

- task_deliver tool (Worker-mode only via DefaultMaskPolicy) accepts
  content + target_filename + citations[]. Routes by extension into
  the renderer pipeline from story 2.6.
- Persists source + rendered artifact, inserts Deliverable row (with
  provenance manifest stub — full manifest in story 2.15), emits
  Misc{kind:"deliverable"} event so the DeliveryRouter picks it up.
- One simplify-and-retry on renderer failure (planner-slot LLM call
  to reduce content complexity). Double-fail falls back to .md raw
  with deliverable_format_fallback Misc.
- Tool catalog count: 37 → 38. spec-check.sh expected count updated.
- 7 unit tests.

refs: /specs/phase-2/stories/story-2.14.md
```

---

## Notes for next story (2.15)

Deliverables flow end-to-end now. 2.15 builds the full provenance
manifest at deliverable-persist time, so the Misc{kind:"deliverable"}
event carries the manifest. After 2.15, the OS-level "provenance
mandatory" invariant from architecture §2.11 holds.
