# Story 1.15 — Narrator Hook (templated + classifier-slot LLM path)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 1.5 (tool-mask layer; AgentMode enum), 1.14 (hook
> output truncation — Narrator emits small bodies but uses the same hook
> framework)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §2.8 (Narrator
> decision matrix), §4.2 (WebSocket `Message.ui:"narrate"` addition),
> §7 (latency + cost budgets), §8 ("Narrator LLM is slow / down"
> failure mode).

---

## Goal

PreToolUse hook that emits a `Message{role:"assistant", ui:"narrate"}`
event for every tool dispatch — using a 0-token templated path for cheap
tools and a ~50-token classifier-slot LLM path for action-changing
tools. Narration is UI signal only; it never re-enters agent context.

## Acceptance criteria

- [ ] `seasoned-hand-core::agent::narrate::NarratorHook` is registered
      as a PreToolUse hook that runs **before** the tool dispatches.
- [ ] Decision matrix (config-keyed, with defaults from architecture
      §2.8):
      - **Templated** (0 tokens): `plan_advance`, `plan_update`,
        `plan_create`, `idle`, `feature_mark_done`, `progress_update`,
        `checkpoint_label`, `file_read`, `file_find_by_name`,
        `file_find_in_content`, `glossary_lookup`, `playbook_search`,
        `sop_read`.
      - **Classifier-slot LLM** (~50 tokens): `file_write`,
        `file_str_replace`, `shell_*`, `browser_*`, `info_search_web`,
        `deploy_*`, `message_notify_user`, `message_ask_user`.
      - Any tool not in either list falls through to **templated**
        with a generic `"Invoking {tool}…"` string.
- [ ] Templated paths use a per-tool string in a Rust `match` (e.g.
      `"Reading {path}"`, `"Marking feature {feature_id} done"`,
      `"Advancing the plan"`). One-line, no period at end (frontend
      adds an ellipsis when streaming).
- [ ] LLM path:
      - Calls the `classifier` slot via the existing `LlmClient`.
      - System prompt loaded from `/config/prompts/narrator.system.txt`
        (file-on-disk, like the verifier prompt).
      - Tool-call **disabled**: `tool_choice: none`, max_tokens ≤ 50.
      - 2-second timeout. On timeout/error: emit Misc
        `narration_skipped{tool, reason}` and proceed without narration.
- [ ] Narration event:
      - `Message { role:"assistant", content: <narration>,
        ui:"narrate", call_id: <pre-dispatch call_id> }`.
      - The sticky-context builder (`build_messages` in `agent::prompt`)
        **filters out** Message events with `ui:"narrate"` so they never
        re-enter the agent's own context.
- [ ] Config:
      ```toml
      [narrator]
      enabled = true
      llm_path = ["file_write", "file_str_replace", "shell_*",
                  "browser_*", "info_search_web", "deploy_*",
                  "message_*"]
      timeout_ms = 2000
      ```
- [ ] Tests:
      - `templated_path_returns_expected_string` — table-driven over
        the ~10 templated tools.
      - `llm_path_calls_classifier_slot` — wiremock'd Bifrost on
        the classifier alias returns a one-line response; assert
        emitted narration matches.
      - `llm_timeout_emits_skipped_misc_and_does_not_block`
        (`tokio::time::pause` to fast-forward).
      - `narration_excluded_from_agent_context` — `build_messages`
        receives a stream with 3 narration Message events and emits no
        Message-role-assistant for them.
      - `disabled_config_emits_no_narration` — `enabled = false` yields
        no Message events.
      - `unknown_tool_falls_through_to_templated_generic`.

## Non-goals

- Streaming narration to the UI character-by-character — frontend lane
  renders the complete narration on event arrival (story 1.18).
- Auto-translation / localization of narration — English only in
  Phase 1.
- Adapting templated strings based on prior narrations (Phase 4+).
- Caching classifier responses across identical tool calls (Phase 4
  optimization).

## Implementation steps

### 1. Module

```
crates/seasoned-hand-core/src/agent/narrate/
  mod.rs          — NarratorHook, NarrationConfig
  templates.rs    — per-tool templated strings + generic fallback
  classifier.rs   — LLM-path call against the classifier slot
  tests.rs
config/prompts/narrator.system.txt  — system prompt for classifier slot
```

### 2. Hook

```rust
pub struct NarratorHook {
    config: NarrationConfig,
    llm: Arc<LlmClient>,
    classifier_slot: ResolvedSlot,
    events: Arc<dyn EventStore>,
    system_prompt: Arc<String>,
}

#[async_trait]
impl PreToolUseHook for NarratorHook {
    async fn on_pre_tool(&self, ctx: &HookContext, args: &Value) {
        if !self.config.enabled { return; }
        let tool = ctx.tool_name.as_str();
        let text = if uses_llm_path(tool, &self.config) {
            match self.classify(ctx, tool, args).await {
                Ok(s) => s,
                Err(e) => {
                    self.events.emit_misc(&ctx.session_id, "narration_skipped",
                        json!({"tool": tool, "reason": e.to_string()})).await.ok();
                    return;
                }
            }
        } else {
            template_for(tool, args)
        };
        self.events.emit_message_narrate(&ctx.session_id, &text, &ctx.call_id).await.ok();
    }
}
```

### 3. Classifier path

```rust
async fn classify(&self, ctx: &HookContext, tool: &str, args: &Value) -> Result<String> {
    let req = ChatCompletionRequest {
        model: self.classifier_slot.alias.clone(),
        messages: vec![
            Message::system(self.system_prompt.as_str()),
            Message::user(&format!(
                "Tool: {tool}\nArgs: {}\nWrite ONE 8-15 word narration sentence for the user.",
                serde_json::to_string(args)?
            )),
        ],
        max_tokens: Some(50),
        tool_choice: Some(ToolChoice::None),
        ..Default::default()
    };
    let resp = tokio::time::timeout(
        Duration::from_millis(self.config.timeout_ms),
        self.llm.chat_completion(req),
    ).await
        .map_err(|_| NarratorError::Timeout)?
        .map_err(NarratorError::Llm)?;
    let text = resp.choices.first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_else(|| format!("Invoking {tool}"));
    Ok(text.trim().to_string())
}
```

### 4. Event-stream emit helper

```rust
// crates/seasoned-hand-core/src/events/store.rs
pub async fn emit_message_narrate(
    &self, session_id: &str, content: &str, call_id: &str,
) -> Result<u64> {
    self.emit(Event {
        session_id: session_id.into(),
        payload: EventPayload::Message {
            role: "assistant".into(),
            content: content.into(),
            ui: Some("narrate".into()),
            call_id: Some(call_id.into()),
        },
        ..Default::default()
    }).await
}
```

### 5. Sticky-context filter

In `crates/seasoned-hand-core/src/agent/prompt.rs::build_messages`,
when converting events to messages, **skip** any `Message` event whose
`ui == Some("narrate")`. Add a unit test asserting the filter holds
(architecture.md §12 q2 → resolved here).

### 6. Templates

```rust
// templates.rs
pub fn template_for(tool: &str, args: &Value) -> String {
    use serde_json::Value as V;
    match tool {
        "plan_advance"        => "Advancing the plan".into(),
        "plan_update"         => "Updating the plan".into(),
        "plan_create"         => "Drafting the plan".into(),
        "idle"                => "Wrapping up".into(),
        "feature_mark_done"   => match args.get("feature_id") {
            Some(V::String(id)) => format!("Marking feature {id} done"),
            _                   => "Marking a feature done".into(),
        },
        "progress_update"     => "Logging progress".into(),
        "checkpoint_label"    => "Labeling next checkpoint".into(),
        "file_read"           => match args.get("path") {
            Some(V::String(p)) => format!("Reading {p}"),
            _                   => "Reading a file".into(),
        },
        "file_find_by_name"   => "Searching workspace for a file".into(),
        "file_find_in_content"=> "Searching workspace content".into(),
        "glossary_lookup"     => "Looking up the glossary".into(),
        "playbook_search"     => "Searching playbooks".into(),
        "sop_read"            => "Reading an SOP".into(),
        _                     => format!("Invoking {tool}"),
    }
}
```

### 7. System prompt

`/config/prompts/narrator.system.txt`:

```
You write one-sentence narrations for a user watching an autonomous agent
work. 8-15 words. Imperative voice. Concrete: name the file, the URL, the
target. No technical jargon ("invoke", "execute"). Output only the
sentence, no quotes, no period at the end.
```

### 8. Misc-kind documentation

Append `narration_skipped` to documented kinds. The `narrate` Message
variant is an envelope-level addition (architecture §4.2).

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core agent::narrate::
cargo test -p seasoned-hand-core agent::prompt::tests::narrate_excluded
./scripts/spec-check.sh
```

Live: run a session that performs `file_read /workspace/README.md`. The
WebSocket stream should carry one `Message{role:"assistant",
ui:"narrate", content:"Reading /workspace/README.md"}` event *before*
the Action event.

---

## Files changed

- `crates/seasoned-hand-core/src/agent/narrate/mod.rs` (new)
- `crates/seasoned-hand-core/src/agent/narrate/templates.rs` (new)
- `crates/seasoned-hand-core/src/agent/narrate/classifier.rs` (new)
- `crates/seasoned-hand-core/src/agent/narrate/tests.rs` (new)
- `crates/seasoned-hand-core/src/agent/mod.rs` (modify — `pub mod narrate;`)
- `crates/seasoned-hand-core/src/agent/prompt.rs` (modify — filter
  `ui:"narrate"` messages)
- `crates/seasoned-hand-core/src/events/store.rs` (modify —
  `emit_message_narrate`)
- `crates/seasoned-hand-core/src/events/payload.rs` (modify — `ui` field
  on Message variant; serde rename `ui:"narrate"` supported)
- `crates/seasoned-hand-core/src/dispatch/hooks.rs` (modify — register
  Narrator as PreToolUse)
- `crates/seasoned-hand-server/src/state.rs` (modify — build
  NarratorHook with classifier slot)
- `config/prompts/narrator.system.txt` (new)
- `config/seasoned-hand.toml` (modify — `[narrator]` block)
- `crates/seasoned-hand-core/src/events/misc.rs` (modify — document
  `narration_skipped`)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.8 (decision matrix), §4.2 (WS
  protocol addition), §7 (latency + cost budgets), §8 (failure mode),
  §12 q2 (narration excluded from agent context — resolved here).
- `/specs/01-architecture/decisions/ADR-003-12-slot-model-routing.md`
  (classifier slot usage).

---

## Commit message

```
feat(phase-1): story 1.15 - Narrator Hook (templated + classifier slot)

- agent::narrate::NarratorHook (PreToolUse): emits
  Message{role:"assistant", ui:"narrate", content, call_id} before
  every tool dispatch
- Decision matrix from architecture §2.8: templated (0 tokens) for
  ~10 cheap tools (file_read, plan_*, idle, feature_mark_done, ...);
  classifier-slot LLM (~50 tokens, max 50 max_tokens, tool_choice:none)
  for action-changing tools (file_write, shell_*, browser_*, etc.)
- 2-second timeout on LLM path → Misc narration_skipped + proceed
  without narration (best-effort UI signal, never blocks dispatch)
- build_messages (agent::prompt) filters out ui:"narrate" messages so
  narration never re-enters agent context (architecture §12 q2)
- Configurable via [narrator] block in seasoned-hand.toml
- Narrator system prompt at config/prompts/narrator.system.txt
- 6 unit tests

refs: /specs/phase-1/stories/story-1.15.md
```

---

## Notes for next story (1.16)

The Narrator emits its own UI lane; story 1.16 (3-track Browser
representation backend) uses the same PostBrowserAction hook pattern to
emit Misc `browser_track_c` events with file_ref screenshots. Story
1.18 surfaces the narration lane in the Chat panel; story 1.19 surfaces
the 3-track view in AgentComputer's BrowserTab.

---

## Execution notes

**Spec divergence — Narrator uses the existing `Hook` trait (story
0.10) instead of a new `PreToolUseHook` trait.** The story example
code wrote `impl PreToolUseHook for NarratorHook`. Phase 0 only ships
one `Hook` trait with three lifecycle methods (`pre`/`post`/`failure`)
and the dispatcher already iterates them in registration order; the
NarratorHook implements `Hook::pre` and no-ops `post`/`failure`.
Registered first in the dispatcher so the `Message{ui:"narrate"}`
event lands before `EventEmittingHook` emits the Action event — clean
UI ordering with no new trait surface.

**Spec divergence — narration goes through the event-store JSON path,
not a new `emit_message_narrate` helper.** The story sketched a
`SqliteEventStore::emit_message_narrate(session_id, content, call_id)`
helper. The existing `append(NewEvent { data: json!({...}) })` path
already handles every Message-event shape; adding a typed helper would
be apparatus for one caller. The hook builds the same JSON inline
(`{role, content, ui, call_id}`); the filter in
`agent::prompt::build_messages` keys on `ui == "narrate"` exactly as
it would have keyed on a typed field. The shape is testable via
serde_json round-trips already covered by the 6 acceptance tests.

**Deferred — classifier-slot wiring through `AppState::new`.**
`NarratorHook` is registered at server boot **without** a classifier
(templated path only), so action-changing tools fall through to
`templates::template_for`'s generic `"Invoking {tool}"` sentence. The
hook's `with_classifier(llm, model, system_prompt)` builder is fully
working (covered by `llm_path_calls_classifier_slot` and the timeout
test), but plumbing the prompt + classifier-slot LlmClient through
`AppState::new` would require restructuring the dispatcher
construction order — the dispatcher is currently `Arc`d before any
prompt-loading runs, and ToolDispatcher has no hot-swap path. A
follow-up commit can either (a) accept `Option<ClassifierWiring>` in
`AppState::new`, or (b) split AppState construction into "tools-only"
+ "tools+narrator" phases. Tracked separately so this story stays at
hook semantics + 6 tests + sticky-context filter.

**No `[narrator]` toml block.** Same rationale as story 1.16: defaults
live in `NarrationConfig::default()` (enabled, 2 s timeout, 7-entry
glob list). The builder `with_config` and `with_classifier` are enough
surface for the AppState wiring and tests, and a real config knob can
land later without breaking callers.
