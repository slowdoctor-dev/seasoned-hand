# Story 1.16 — 3-track Browser representation (backend)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 1.14 (hook file-ref path — screenshots routinely
> exceed 16 KB)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §2.7 (3-track
> table), §3.4 (Misc `browser_track_c` payload), §7 (sandbox HTTP call
> cost), §12 q7 (full-resolution; no thumbnailing in Phase 1).

---

## Goal

After every `browser_*` tool invocation, capture three parallel
representations of browser state: Track A (live noVNC — already
running, no backend change), Track B (DOM text snapshot — inline or
file_ref via story 1.14 path), Track C (PNG screenshot saved to
`/workspace/.tracks/<call_id>.png` and emitted as a Misc event with
file_ref). Backend-only; frontend rendering is story 1.19.

## Acceptance criteria

- [ ] `seasoned-hand-core::browser::tracks::PostBrowserActionHook`
      registered as PostToolUse for every tool whose name starts with
      `browser_`.
- [ ] **Track A** — no backend change. Already streamed via the Phase 0
      noVNC iframe; this story documents it but does nothing.
- [ ] **Track B**:
      - If the tool is `browser_view`, the existing return already
        contains DOM text — reuse it.
      - Otherwise, the hook **reuses the same sandbox call path the
        Phase 0 `browser_view` tool dispatches to** (do not invent a
        new HTTP endpoint — find the existing `SandboxClient` method
        that backs the Phase 0 `browser_view` tool and call it
        directly from the hook). This keeps Track B coupled to one
        canonical browser-view code path.
      - The DOM text is attached to the Observation as
        `output.dom_text`. If > 16 KB, story 1.14's helper writes it as
        a file_ref. The Observation event's payload gains a
        `dom_text_ref: { inline?: String, file?: FileRef }` field.
- [ ] **Track C**:
      - Hook calls sandbox screenshot endpoint
        (`/v1/browser/screenshot`) to capture a PNG.
      - Writes the PNG to `/workspace/.tracks/<call_id>.png` via the
        SandboxClient.
      - Emits `Misc{kind:"browser_track_c", data: {call_id,
        file_ref: {path, sha256, size}}}`.
- [ ] Cost: one extra sandbox HTTP call per browser tool invocation for
      the screenshot (Track B reuses the view call when the tool is
      already `browser_view`).
- [ ] Failure modes:
      - Screenshot endpoint times out (3 s) → emit Misc
        `browser_track_c_skipped{reason:"timeout", call_id}`. Loop
        continues.
      - Sandbox file_write fails → same skip pattern with
        `reason:"sandbox_write_failed"`.
      - DOM-text capture fails → Observation still emitted *without*
        the `dom_text_ref` field; emit Misc `browser_track_b_skipped`.
- [ ] Tests:
      - `browser_view_reuses_dom_text` — when the dispatched tool is
        `browser_view`, assert the hook does NOT invoke the
        SandboxClient's view accessor a second time (counter on the
        mock asserts `view_calls == 1` for that tool dispatch).
      - `browser_click_captures_both_tracks_b_and_c` — wiremock'd
        sandbox returns canned DOM text + PNG; assert one Observation
        with `dom_text_ref` and one Misc `browser_track_c`.
      - `large_dom_text_becomes_file_ref` — synthetic 50 KB DOM text;
        assert FileRef body.
      - `screenshot_timeout_emits_skipped_misc` (`tokio::time::pause`).
      - `non_browser_tool_does_not_trigger_hook`.
      - `track_c_filename_matches_call_id` —
        `/workspace/.tracks/<call_id>.png` actually written.

## Non-goals

- Frontend rendering (story 1.19).
- Thumbnailing or per-track retention (phase-1/DEBT.md #8 — tied to
  Phase 0 DEBT #16).
- Captures for non-browser tools.
- Pagination / cursoring of the screenshot strip — frontend does this
  in 1.19.

## Implementation steps

### 1. Hook module

```
crates/seasoned-hand-core/src/browser/tracks/
  mod.rs        — PostBrowserActionHook
  capture.rs    — DOM-text + screenshot helpers
  tests.rs
```

### 2. Hook

```rust
pub struct PostBrowserActionHook {
    sandbox: Arc<SandboxClient>,
    events: Arc<dyn EventStore>,
    screenshot_timeout: Duration,
}

#[async_trait]
impl PostToolUseHook for PostBrowserActionHook {
    async fn on_post_tool(&self, ctx: &HookContext, obs: &mut Observation) {
        if !ctx.tool_name.starts_with("browser_") { return; }

        // Track B
        let dom_text = if ctx.tool_name == "browser_view" {
            obs.output.get("text").and_then(Value::as_str).map(String::from)
        } else {
            match self.sandbox.browser_view(&ctx.session_id).await {
                Ok(t) => Some(t),
                Err(e) => {
                    self.events.emit_misc(&ctx.session_id, "browser_track_b_skipped",
                        json!({"call_id": ctx.call_id, "reason": e.to_string()})).await.ok();
                    None
                }
            }
        };
        if let Some(t) = dom_text {
            let payload = events::truncation::write_large_or_inline(
                &self.sandbox, &ctx.session_id, ctx.event_id_hint(),
                t.as_bytes(), "text/plain"
            ).await.ok();
            obs.attach_dom_text_ref(payload);
        }

        // Track C
        let png = match tokio::time::timeout(self.screenshot_timeout,
            self.sandbox.browser_screenshot(&ctx.session_id)
        ).await {
            Ok(Ok(b)) => b,
            Ok(Err(e)) => {
                self.events.emit_misc(&ctx.session_id, "browser_track_c_skipped",
                    json!({"call_id": ctx.call_id, "reason": e.to_string()})).await.ok();
                return;
            }
            Err(_) => {
                self.events.emit_misc(&ctx.session_id, "browser_track_c_skipped",
                    json!({"call_id": ctx.call_id, "reason": "timeout"})).await.ok();
                return;
            }
        };
        let path = format!("/workspace/.tracks/{}.png", ctx.call_id);
        if let Err(e) = self.sandbox.write_workspace_file(&ctx.session_id, &path, &png).await {
            self.events.emit_misc(&ctx.session_id, "browser_track_c_skipped",
                json!({"call_id": ctx.call_id, "reason": format!("sandbox_write_failed: {e}")})).await.ok();
            return;
        }
        let sha256 = format!("{:x}", sha2::Sha256::digest(&png));
        self.events.emit_misc(&ctx.session_id, "browser_track_c", json!({
            "call_id": ctx.call_id,
            "file_ref": { "path": path, "sha256": sha256, "size": png.len(),
                          "content_type": "image/png" },
        })).await.ok();
    }
}
```

### 3. SandboxClient additions

```rust
impl SandboxClient {
    // browser_view: the Phase 0 `browser_view` tool already dispatches
    // to a sandbox method. Either reuse that exact method here (if its
    // visibility allows) or extract a `pub(crate)` helper from it so
    // both the tool and the hook share one implementation. Do NOT add
    // a parallel HTTP path with the same purpose.
    pub async fn browser_view(&self, session_id: &str) -> Result<String, SandboxError> { ... }

    // browser_screenshot: this IS new in Phase 1. Calls AIO Sandbox's
    // existing screenshot endpoint per its upstream docs.
    pub async fn browser_screenshot(&self, session_id: &str) -> Result<Vec<u8>, SandboxError> { ... }
}
```

`browser_screenshot` is new; `browser_view` is a shared accessor over
the Phase 0 tool's existing sandbox call. Confirm the exact endpoint
name during implementation by reading `crates/seasoned-hand-core/src/tools/browser_view.rs`
(Phase 0 story 0.9 / 0.9b) — do NOT guess.

### 4. Observation payload extension

`Observation::dom_text_ref: Option<EventPayloadBody>` added as a serde
field. Backwards-compatible: missing field = `None`. The Verifier
context builder (story 1.9) reads this field when constructing evidence
for browser-related triggers.

### 5. Configuration

```toml
[browser.tracks]
screenshot_timeout_ms = 3000
```

### 6. Misc-kind documentation

Append `browser_track_c, browser_track_c_skipped, browser_track_b_skipped`.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core browser::tracks::
./scripts/spec-check.sh
```

Live (sandbox required): start a session, call `browser_navigate
https://example.org`. Confirm the event stream shows one Action +
one Observation (with `dom_text_ref`) + one Misc `browser_track_c`. The
file `/workspace/.tracks/<call_id>.png` exists in the sandbox.

---

## Files changed

- `crates/seasoned-hand-core/src/browser/tracks/mod.rs` (new)
- `crates/seasoned-hand-core/src/browser/tracks/capture.rs` (new)
- `crates/seasoned-hand-core/src/browser/tracks/tests.rs` (new)
- `crates/seasoned-hand-core/src/browser/mod.rs` (modify or new — `pub
  mod tracks;`)
- `crates/seasoned-hand-core/src/sandbox/client.rs` (modify —
  `browser_view`, `browser_screenshot` helpers)
- `crates/seasoned-hand-core/src/events/payload.rs` (modify —
  `Observation::dom_text_ref` field)
- `crates/seasoned-hand-core/src/dispatch/hooks.rs` (modify — register
  `PostBrowserActionHook`)
- `crates/seasoned-hand-core/src/events/misc.rs` (modify — document
  new kinds)
- `config/seasoned-hand.toml` (modify — `[browser.tracks]`)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.7 (verbatim table), §3.4 (Misc
  payloads), §7 (cost), §12 q7 (Phase 1 stores full-resolution).
- `/specs/phase-1/DEBT.md` #8 (retention deferred).

---

## Commit message

```
feat(phase-1): story 1.16 - 3-track browser representation (backend)

- PostBrowserActionHook (browser::tracks) runs after every browser_*
  tool dispatch
- Track A: no change (Phase 0 noVNC iframe)
- Track B: DOM text. For browser_view, reuse the tool's return. For
  other browser_* tools, the hook reuses the SAME sandbox accessor
  the Phase 0 browser_view tool dispatches to (do not invent a new
  HTTP path — find the existing method via the Phase 0 tool source
  and share it). Attached to Observation as dom_text_ref; >16KB uses
  the story-1.14 file-ref helper
- Track C: PNG screenshot via /v1/browser/screenshot (3-second
  timeout), written to /workspace/.tracks/<call_id>.png; emits
  Misc browser_track_c{call_id, file_ref{path, sha256, size}}
- Failure modes for B and C surface as browser_track_*_skipped Misc
  events; the agent loop never blocks on capture
- SandboxClient gains browser_view + browser_screenshot helpers
- 6 unit + integration tests

refs: /specs/phase-1/stories/story-1.16.md
```

---

## Notes for next story (1.17)

The backend now produces three browser-track streams. Story 1.17 (WS
task control) is independent of this backend work and can be done in
parallel. Story 1.19 (frontend BrowserTab) consumes:

- Track A: existing noVNC iframe (Phase 0).
- Track B: `Observation.dom_text_ref` (resolved on-demand by frontend).
- Track C: `Misc{kind:"browser_track_c"}` events, file path served
  through `/v1/workspace/:session_id/*` Phase 0 route.
