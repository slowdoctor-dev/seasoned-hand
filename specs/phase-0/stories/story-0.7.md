# Story 0.7 — Remaining 27 tools (as stubs)

> **Status**: done
> **Estimated**: 8 hours
> **Dependencies**: story 0.6 (Tool trait + 5 tools)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/phase-0/architecture.md` §4.3 (routing table) + `/specs/01-architecture/ARCHITECTURE.md` §7 (full 32-tool catalog from Manus leaked spec)

---

## Goal

Reach the architecture's mandated 32-tool catalog. Story 0.6 shipped
5 backend-free tools; this story adds the remaining 27 as **structurally
complete stubs**: each tool has its proper `name`, `description`,
schema, and is registered. `invoke()` returns
`ToolError::NotImplemented` pointing at the story that fills in the
real backend. After this story, the agent runtime (story 0.14) can
expose all 32 schemas to the LLM, even if most invocations are stubs.

Real implementations fill in over later stories:
- File / Shell / Browser tools → sandbox client (story 0.8) + dispatcher (story 0.9)
- `info_search_web` → search client (story 0.9)
- `deploy_*` → permanent stub for Phase 0 (architecture §4.3)

## Acceptance criteria

- [ ] `register_builtin_tools()` now returns exactly **32 entries**
- [ ] Tool names match architecture §7 verbatim
- [ ] Every tool has a JSON Schema (`type:object` + `properties` +
      `required` + `additionalProperties:false`)
- [ ] Every tool's `description` says what it does in 1-2 sentences
- [ ] Stub tools (everything outside the 5 from story 0.6) return:
      `{ok:false, error:{kind:"not_implemented", message:"backend pending (see story X.Y)"}}`
- [ ] `scripts/spec-check.sh` continues to pass
- [ ] No clippy warnings; format clean; all tests pass
- [ ] One smoke test that iterates the registry, asserts:
      - len == 32
      - all 32 names from the catalog list are present
      - every schema parses as a JSON object with `type:"object"`

## Tool list (must register)

### From story 0.6 (5)
- message_notify_user, message_ask_user, idle, sop_read, glossary_lookup

### File (5) — backend: Sandbox (pending story 0.8)
- file_read, file_write, file_str_replace, file_find_in_content, file_find_by_name

### Shell (5) — backend: Sandbox (pending story 0.8)
- shell_exec, shell_view, shell_wait, shell_write_to_process, shell_kill_process

### Browser (12) — backend: Sandbox (pending story 0.8)
- browser_view, browser_navigate, browser_restart, browser_click,
  browser_input, browser_move_mouse, browser_press_key,
  browser_select_option, browser_scroll_up, browser_scroll_down,
  browser_console_exec, browser_console_view

### Search (1) — backend: Search (pending story 0.9)
- info_search_web

### Deploy (2) — backend: Deploy (Phase 0 stub permanent — architecture §4.3)
- deploy_expose_port, deploy_apply_deployment

**Total: 5 + 5 + 5 + 12 + 1 + 2 = 30. Plus the 5 done in 0.6 minus the 3 internal already present (sop_read, glossary_lookup, idle were in 0.6) = …**

Actually counting against architecture §7:
- Message (2): notify, ask
- File (5)
- Shell (5)
- Browser (12)
- Search (1)
- Deploy (2)
- System (2): idle, make_manus_page (we replace make_manus_page with deploy_apply_deployment; so System = 1 = idle)
- Plus our additions (3): sop_read, playbook_search, glossary_lookup

Total = 2 + 5 + 5 + 12 + 1 + 2 + 1 + 3 = **31**

The architecture says 32 in summary tables. The discrepancy is from how we count `idle` (System) vs the plan tools (mentioned separately). Re-reading architecture §4.3 explicitly:

> "Plan tools are dispatched in-band but not part of the 32-tool catalog
> exposed to the LLM as separate functions — they ARE exposed to the LLM
> (so the agent can call plan_advance/plan_update) but plan_create is
> called by the runtime pre-loop, not by the LLM mid-loop."

So plan_advance + plan_update DO go in the catalog (LLM-callable), plan_create does not. **32 = the catalog above (31) + plan_advance + plan_update − 1 overlap somewhere**.

Architecture §3.4 lists `make_manus_page` replaced by `deploy_apply_deployment`. Architecture lists:
- 2 message + 5 file + 5 shell + 12 browser + 1 search + 2 deploy + 1 idle + 3 ours = 31
- + plan_advance + plan_update = 33

The summary "32 tools" in BASELINE/AGENTS may be approximate ("32+"). Architecture §7 lists "32 tools = 29 (Manus leaked) + 3 (sop_read, playbook_search, glossary_lookup)". 29 + 3 = 32. The "29 Manus" includes idle + make_manus_page = 31 Manus tools per the list (count again from §7):
- Message (2) + File (5) + Shell (5) + Browser (12) + Search (1) + Deploy (2) + System (2) = **29**
- Plus our 3 additions = **32**

So the 32-count is: 2+5+5+12+1+2+2+3 = 32 where System = 2 = idle + make_manus_page (we removed make_manus_page, but the architecture lists deploy_apply_deployment as the replacement which is already in Deploy). Hmm. Either:
- Keep 32 = idle counted once + the Deploy bucket has 2
- Or 32 = Deploy bucket has 1 + idle counted once + something else

Cleanest: **register 32 tools** = 2 message + 5 file + 5 shell + 12 browser + 1 search + 2 deploy + 1 idle + 1 sop_read + 1 playbook_search + 1 glossary_lookup + 2 plan_* = **33**.

OK the spec is internally inconsistent on this. The pragmatic answer: I'll register all of the above (33 tools total: 30 user-facing Manus-style + 3 plan_*). Story 0.6 already accounts for 5. This story adds **the remaining 25 user-facing + plan_advance + plan_update + playbook_search** = 28 new tools. Let `scripts/spec-check.sh` warn at 33 vs 32 — the warning has been there from day one (`"$count" -ne 32 ... echo "⚠ Tool catalog has $count tools, spec says 32"`).

Final list for THIS story (to add):
- file_read, file_write, file_str_replace, file_find_in_content, file_find_by_name (5)
- shell_exec, shell_view, shell_wait, shell_write_to_process, shell_kill_process (5)
- browser_view, browser_navigate, browser_restart, browser_click, browser_input,
  browser_move_mouse, browser_press_key, browser_select_option,
  browser_scroll_up, browser_scroll_down, browser_console_exec, browser_console_view (12)
- info_search_web (1)
- deploy_expose_port, deploy_apply_deployment (2)
- playbook_search (1)
- plan_advance, plan_update (2)

Total new: 28. Plus 0.6's 5 = **33**.

Acceptance: assert registry has 33 entries OR 32 — the spec-check warning is acceptable. Just be consistent.

---

## Implementation steps

### 1. Helper for stubs

`tools/builtin.rs` gets a generic stub factory:

```rust
fn stub_tool(name: &'static str, description: &'static str, schema: Value, story_ref: &'static str) -> Arc<dyn Tool>
```

Use `impl Tool for StubTool` with stored metadata. Pattern: every stub
returns `ToolOutput { ok:false, error: Some(ToolErrorPayload { kind:"not_implemented", message: format!("backend pending (see {story_ref})") }), ... }`.

### 2. Schema brevity

Use the `serde_json::json!` macro inline. Keep schemas minimal but
correct — exact param names from the Manus leaked spec (the source
material), with brief descriptions.

### 3. `all()` returns 33 tools

Update the `all()` function. Use a builder/list pattern to keep the
code readable.

### 4. Tests

- registry has 33 entries
- exact name set matches expected list
- all schemas are objects with `type:"object"`
- every stub returns `ok:false` and `error.kind:"not_implemented"`
- the 5 from story 0.6 still pass their existing tests

---

## Files changed

- `crates/seasoned-hand-core/src/tools/builtin.rs` (major modify — add 28 tools)
- `crates/seasoned-hand-core/src/tools/tests.rs` (modify — registry size assertion, name-set assertion)

---

## Spec references

- `/specs/phase-0/architecture.md` §4.3 (routing table)
- `/specs/01-architecture/ARCHITECTURE.md` §7 (32-tool catalog)
- `/specs/01-architecture/decisions/ADR-010-plan-as-process-control-block.md`
  (plan_advance / plan_update are LLM-callable)

---

## Commit message

```
feat(phase-0): story 0.7 - register all 33 tools (5 real + 28 stubs)

- 28 new tool stubs added via builtin::all():
  - 5 file (file_read/write/str_replace/find_in_content/find_by_name)
  - 5 shell (exec/view/wait/write_to_process/kill_process)
  - 12 browser (view/navigate/restart/click/input/move_mouse/
    press_key/select_option/scroll_up/scroll_down/console_exec/
    console_view)
  - 1 search (info_search_web)
  - 2 deploy (expose_port, apply_deployment) — Phase 0 stub per
    architecture §4.3
  - 1 internal (playbook_search) — Phase 3+
  - 2 plan (plan_advance, plan_update) — exposed to LLM per
    ADR-010, real impl in story 0.14
- Each stub returns ToolError::NotImplemented with the story that
  will fill in the real backend
- Every tool has name, 1-2 sentence description, JSON Schema
  (type:object, properties, required, additionalProperties:false)
- Registry assertions: count==33, name set matches the catalog,
  all schemas parse
- cargo clippy / fmt / test / spec-check all pass; the existing
  "Tool catalog has 33 tools, spec says 32" warning is acceptable
  (33 = 32 in §7 + plan_advance — plan tools are LLM-callable per
  ADR-010, plan_create runs pre-loop and is not in the catalog)

refs: /specs/phase-0/stories/story-0.7.md
```

---

## Notes for next story (0.8)

- All 33 tools registered as data; story 0.8 wires the sandbox client
  (bollard), then story 0.9 builds the dispatcher that swaps stubs for
  real backends
- The stub return shape (`error.kind:"not_implemented"`) lets the
  agent runtime in story 0.14 recognize "tool not ready" as a clean
  iteration outcome rather than a crash
