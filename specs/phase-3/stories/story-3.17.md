# Story 3.17 — Production ExtractionHandler + main.rs wiring (Phase 3 BLOCKER close-out)

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.8 (full Phase 3 backend slice)
> **Phase**: 3
> **Type**: backend+test
> **Origin**: REVIEW iter-3 (Claude, `4988aa7`) finding A1 + DEBT #84 (Phase 4 BLOCKER)

---

## Goal

Ship the production `ExtractionHandler` impl that makes Phase 3's headline
learning loop actually run end-to-end. Currently (post-3.16) the Phase 3
acceptance gate passes by acceptance-criteria letter, but every PASS task in
production emits `Misc{kind:"playbook_extraction_error", stage:"llm_call",
reason:"extraction_handler_not_configured"}` and writes no playbook — the loop
is scaffolded but never closes. This story builds the missing handler, wires
it into `seasoned-hand-server/src/main.rs`, and adds an end-to-end test that
drives a stub LLM through the full extract → match → inject → counter-update
loop so future regressions surface immediately.

## Acceptance criteria

- [ ] A new `PlannerSlotExtractionHandler` (or similarly named) production
      implementation of `crate::verifier::gate::ExtractionHandler` lives under
      `crates/seasoned-hand-core/src/verifier/` (or `src/learning/` if the
      author prefers a new module).
- [ ] The handler:
      - resolves `SlotName::Planner` via the injected `SlotRouter`,
      - builds the extraction prompt with F-3.13 layer-1 refusal guidance +
        F-3.14 layer-1 abstraction guidance baked into the system message,
      - calls the LLM (`LlmClient::chat_completion`) for structured JSON
        output `{title, trigger_keywords: string[], overview, steps: string[]}`,
      - applies in order: F-3.14 layer-2 deterministic redaction →
        F-3.13 layer-2 deterministic adversarial scan → F-3.18 quality-floor
        validator (uses existing helpers from `verifier/extraction.rs`),
      - renders `content = overview + "\n\n## Procedure\n" + numbered(steps)`,
      - applies NFR-3.5 24_576-byte cap, emits the combined
        `playbook_extraction_output_capped` + `playbook_extraction_rejected{layer:"quality_floor"}`
        events when post-cap content drops below floor (architecture §3 step 6),
      - inserts the row into `playbooks` (project-scoped via
        `source_task_id`, `status='active'`, `version=1`), letting the
        existing V010 FTS5 triggers index it.
- [ ] `seasoned-hand-server/src/main.rs:346` is updated to construct the
      handler and call `.with_extraction(Arc::new(handler))` on `VerifierGate`.
      A `SH_LEARNING_ENABLED` env var (default `true`) lets operators disable
      the handler without recompile (production safety valve).
- [ ] End-to-end integration test under
      `crates/seasoned-hand-core/src/verifier/` (gated `#[cfg(test)]`) that:
      - seeds a synthetic Brief + agent-loop transcript,
      - stubs the planner LLM to return a known structured JSON response,
      - calls the production handler via `VerifierGate.run_sync_extraction(...)`,
      - asserts a playbook row appears with the expected
        `title`/`trigger_keywords`/`content`/`status='active'`,
      - asserts the playbook is now findable by `match_playbooks(...,
        MatcherMode::Production)` and gets injected by `inject_playbooks(...)`,
      - asserts no `playbook_extraction_error` events for the success path.
- [ ] `phase3_warm_benchmark` (in `verifier/gate.rs`) is updated to drive
      a real cold → warm sequence through the production handler with the
      stub LLM, asserting the warm session's `sessions.tool_calls` is genuinely
      reduced via injection — not seeded via direct Action-event count. This
      closes DEBT #85 (warm benchmark scenario-driven, not loop-driven).
- [ ] Adversarial path test: stub LLM returns a response with shell metachars
      or prompt-injection phrases; assert `playbook_extraction_rejected{layer:"deterministic"}`
      fires and no playbook row is written.
- [ ] PII redaction path test: stub LLM returns a response with email + IP +
      bearer header; assert `playbook_extraction_pii_redacted{layer:"deterministic"}`
      fires AND the written playbook content has the redacted markers.

## Non-goals

- Async workerization of extraction (Phase 4 Curator scope per F-3.7).
- Embedding-based rerank (Phase 4, DEBT #72).
- Auto-archive / consolidation policies (Phase 4 Curator, F-3.9).
- Curator auto-tuning of FTS weights / recency decay (Phase 4, DEBT #76).

---

## Implementation steps

1. **Author the handler module** (~150 lines):
   `crates/seasoned-hand-core/src/verifier/extraction_handler.rs` —
   `PlannerSlotExtractionHandler` struct + `impl ExtractionHandler`.
   Constructor takes `SlotRouter` + `Arc<DbPool>`.
2. **Define the structured-output JSON schema** in the system message; use
   `temperature=0.0` for determinism + `max_tokens` sized to fit NFR-3.5 cap.
3. **Chain the helper functions** from `verifier/extraction.rs` in the order
   specified by architecture §3 7-step pipeline.
4. **Write the inserts** with `INSERT INTO playbooks(...)` using `params!` —
   match the V010 column shape exactly. Let the FTS5 triggers do the indexing.
5. **Wire into `seasoned-hand-server/src/main.rs:346`**: construct the handler,
   gate behind `SH_LEARNING_ENABLED`, attach via `.with_extraction(...)`.
6. **Add the 4 tests** specified in acceptance criteria.
7. **Update `phase3_warm_benchmark`** to drive the real loop (closes DEBT #85).
8. **Run the full AGENTS.md §6 gate list** before commit.

---

## Verification

```bash
# Full AGENTS.md §6 gate list (required per REVIEW iter-1 F7)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
pnpm typecheck
pnpm test
bash scripts/spec-check.sh

# Story-3.17-specific evidence
cargo test -p seasoned-hand-core extraction_handler::end_to_end_loop
cargo test -p seasoned-hand-core extraction_handler::adversarial_rejection
cargo test -p seasoned-hand-core extraction_handler::pii_redacted
cargo test phase3_warm_benchmark
```

---

## Refs

- requirements: F-3.1, F-3.5, F-3.7, F-3.8, F-3.11, F-3.13, F-3.14, F-3.15, F-3.18, NFR-3.1, NFR-3.5
- architecture: §2.1 LearningExtractor, §3 7-step pipeline, §4 (handler API surface)
- review: /specs/phase-3/REVIEW.md iter-3 A1
- debt closed by this story: #84 (Phase 4 BLOCKER → resolved), #85 (warm benchmark loop-driven)

## Notes

This story was authored AFTER the original PM persona's 16-story breakdown
because REVIEW iter-3 discovered the structural gap. Future PM passes
(Phase 4+) should ensure each scaffolded interface has a paired story shipping
a production impl, not just a test impl.
