# Story 0.9b — Wire remaining 18 sandbox tools (follow-up to 0.9)

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 0.8 (sandbox client), 0.9 (dispatcher + sandbox_post helper)
> **Phase**: 0 (closeout)
> **Type**: backend
> **Reads first**: `/specs/phase-0/stories/story-0.9.md` (the 4 done + the pattern), `/specs/phase-0/architecture.md` §4.3 routing table, `/specs/phase-0/DEBT.md` #19

---

## Goal

Replace the 18 `StubTool` entries from story 0.7 with real
`sandbox_post`-based implementations so the agent can actually browse,
edit files, and inspect shells in the AIO Sandbox container. This is
the bottleneck blocking `tests/e2e_phase0.rs` from being un-`#[ignore]`'d
— the acceptance task ("find GitHub stars of FoundationAgents/OpenManus")
needs `browser_navigate` + `browser_view` to be real.

## Acceptance criteria

- [ ] 18 stub tools in `crates/seasoned-hand-core/src/tools/builtin.rs`
      replaced with real `Tool` impls using the existing `sandbox_post`
      helper (defined in 0.9):
      - **File (3)**: `file_str_replace`, `file_find_in_content`, `file_find_by_name`
      - **Shell (4)**: `shell_view`, `shell_wait`, `shell_write_to_process`, `shell_kill_process`
      - **Browser (12)**: `browser_view`, `browser_navigate`, `browser_restart`,
        `browser_click`, `browser_input`, `browser_move_mouse`, `browser_press_key`,
        `browser_select_option`, `browser_scroll_up`, `browser_scroll_down`,
        `browser_console_exec`, `browser_console_view`
- [ ] Each impl matches the AIO Sandbox v1.0.0.152 HTTP API. Reference:
      `https://github.com/agent-infra/sandbox` SDK paths.
      Expected mapping (implementer verifies field names against the
      pinned image's docs/handlers):
      - File: `POST /v1/file/{replace|grep|find}` (or `glob` if `find`
        doesn't exist for the glob case)
      - Shell: `POST /v1/shell/{view|wait|write|kill}`
      - Browser: most multiplex through `POST /v1/browser/actions` with
        an `{action: "<type>", ...}` body; `browser_restart` is its own
        `POST /v1/browser/restart`; `browser_view` likely
        `POST /v1/browser/screenshot` or combines screenshot + info
- [ ] If the AIO Sandbox API doesn't expose a 1:1 endpoint for a given
      tool, the impl returns `ToolError::NotImplemented("phase 1: …")`
      and adds a focused DEBT entry. **Don't fake the wire format.**
- [ ] Update the registry assertion in `tools/tests.rs`: the
      `stubs_return_not_implemented` test's `real` allowlist gains
      these 18 names (so it doesn't false-positive)
- [ ] Add one round-trip unit test per backend group via wiremock:
      - `file_str_replace_posts_replace_body` (path + old_str + new_str)
      - `shell_view_posts_process_id`
      - `browser_navigate_posts_url_action`
      One test per group is enough — they all share `sandbox_post` so
      the per-tool wiring is mechanical
- [ ] DEBT.md: close #19 with the date and the resulting follow-ups
      (any tool that genuinely doesn't have an AIO endpoint becomes
      its own entry)
- [ ] `cargo clippy --all-targets -- -D warnings` passes (cold cache,
      not just incremental — the Phase 0 retrospective caught this hole)
- [ ] `cargo fmt --check / cargo test --workspace / ./scripts/spec-check.sh`
      pass
- [ ] **Spec-check gate update**: `scripts/spec-check.sh`'s tool-count
      check already asserts 33; no change needed (registry count stays
      the same — only the StubTool→real swap)

## Non-goals

- Re-enabling `tests/e2e_phase0.rs` (separate follow-up — needs CI from
  DEBT #14 + live Bifrost + provider keys; do once both this story and
  the CI story land)
- Browser action body validation against a live Chromium (smoke test
  only; the agent will validate at first real run)
- `playbook_search` / `sop_read` / `glossary_lookup` real impls (those
  are Phase 3+ per architecture)
- `deploy_expose_port` / `deploy_apply_deployment` (Phase 1+ per
  architecture §4.3)

---

## Implementation steps

### 1. Verify AIO Sandbox API field names

Before writing any tool, pull one example from the SDK to ground the
body shapes:

```bash
# from a clean curl or rg over node_modules / SDK files for each path
curl -s https://raw.githubusercontent.com/agent-infra/sandbox/main/sdk/python/agent_sandbox/file/raw_client.py | rg -A8 "v1/file/replace"
```

Confirm: does `/v1/file/replace` take `{file, old_str, new_str}` or
`{path, old_str, new_str}`? Same for `/v1/shell/view` — does it take
`{process_id}` or `{session_id}`? Adjust the impl bodies accordingly.

### 2. File tools (3) — small additions

Mirror the `FileRead`/`FileWrite` structure from 0.9. One struct per
tool, each calling `sandbox_post(ctx, "/v1/file/<verb>", body)`.

### 3. Shell tools (4) — small additions

Same pattern. `process_id` is the common arg.

### 4. Browser tools (12) — wrapper helper

Twelve browser tools all funnel through `POST /v1/browser/actions` with
different `{action: "<name>", ...args}` bodies. Add a private helper:

```rust
async fn browser_action(ctx: &ToolContext, action: &str, body: Value)
    -> Result<ToolOutput, ToolError>;
```

It merges `{"action": action}` into `body` then calls
`sandbox_post(ctx, "/v1/browser/actions", merged)`. Each of the 12
tools is then a thin shim.

`browser_restart` is the one exception — its own endpoint
`/v1/browser/restart` with empty body.

`browser_view` likely needs two calls (screenshot + DOM) — the spec
allows one combined call if the endpoint supports it. If it requires
two, the impl can fan them out and return a merged JSON output.

### 5. Update registry + tests

In `tools/builtin.rs::all()`, replace the 18 `stub(...)` lines with
`Arc::new(<NewStructName>)`. Keep the existing struct + Tool impl
order so diffs read cleanly.

In `tools/tests.rs`, append the 18 names to the `real` allowlist.

Add 3 round-trip wiremock tests as listed above.

### 6. Close DEBT #19

Strike-through with date in `specs/phase-0/DEBT.md`. If any of the 18
tools end up returning `NotImplemented` because the AIO Sandbox API
doesn't expose them, open a new numbered DEBT entry per missing endpoint.

---

## Files changed

- `crates/seasoned-hand-core/src/tools/builtin.rs` (modify — 18 stubs
  → real, +1 helper `browser_action`)
- `crates/seasoned-hand-core/src/tools/tests.rs` (modify — extend `real`
  allowlist, +3 wiremock tests)
- `specs/phase-0/DEBT.md` (close #19, possibly open follow-ups)

---

## Spec references

- `/specs/phase-0/stories/story-0.9.md` (the pattern + `sandbox_post`)
- `/specs/phase-0/stories/story-0.7.md` (the 33-tool catalog + EXPECTED_TOOLS list)
- `/specs/phase-0/architecture.md` §4.3 (routing table)
- `agent-infra/sandbox` GitHub repo SDK for actual API field names

---

## Commit message

```
feat(phase-0): story 0.9b - wire remaining 18 sandbox tools

- 3 file (str_replace, find_in_content, find_by_name)
- 4 shell (view, wait, write_to_process, kill_process)
- 12 browser (multiplexed through /v1/browser/actions with action-
  typed bodies; restart on its own endpoint)
- browser_action() helper centralizes the action-body merging
- registry stays 33 entries; stubs_return_not_implemented allowlist
  extended
- 3 wiremock round-trip tests (one per backend group)
- DEBT #19 closed (any AIO-API-missing tool becomes a focused
  follow-up entry)
- cargo clippy/fmt/test/spec-check all pass

After this lands, tests/e2e_phase0.rs can finally exercise its
acceptance criterion once DEBT #14 (CI) is in place and live
Bifrost + provider keys are configured.

refs: /specs/phase-0/stories/story-0.9b.md
```

---

## Notes for next step

The two remaining Phase-0-closeout stories pair naturally:

- **story-0.14-ci.md** (or just take DEBT #14's name) — `.github/workflows/ci.yml`
  that brings up Bifrost + Redis containers, runs all gates from cold
  cache, and runs the ignored E2E + Redis + sandbox-lifecycle tests
  if the secrets exist. Once both this story and the CI story land,
  the `#[ignore]` on `tests/e2e_phase0.rs` can be removed and Phase 0
  is truly closed.
