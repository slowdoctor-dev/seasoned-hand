# Phase 3 — Learning System

> Status: v1.0 (planning)
> Duration: 4 weeks (ROADMAP baseline)
> Goal: Make the learning claim operational: for a deterministic employee-workflow
> benchmark, the second run should complete with materially fewer tool calls while the
> learning pipeline remains general-purpose (not benchmark-overfit), safe-by-default,
> and operable by humans through CLI lifecycle surfaces.

## 1. Goals (what success looks like)

- Prove measurable time-axis improvement on a deterministic benchmark: second run of
  the same workflow shape uses <=70% of cold baseline tool calls.
- Ship a real learning loop, not a gate-only trick: extraction trigger remains
  content-agnostic across verified tasks, production matching works beyond benchmark
  identity matching, and extracted artifacts become reusable immediately.
- Keep Phase 3/4 boundary explicit: Phase 3 extracts, matches, injects, records
  counters/telemetry; Phase 4 Curator decides archive/consolidate/rate policy.
- Keep Phase 3/5 boundary explicit: Phase 3 is single-operator + project-scoped; full
  tenant isolation and cross-tenant safety workflows remain Phase 5 scope.

## 2. Non-functional requirements

- **NFR-3.1 (sync extraction latency bound)**: Synchronous extraction must complete
  within 60 seconds of `task_complete`. Timeout behavior must emit
  `Misc{kind:"playbook_extraction_timeout", session_id, elapsed_ms}` and skip
  playbook write; task completion is never blocked indefinitely.
- **NFR-3.2 (injection determinism)**: Playbook injection at task start adds no extra
  LLM round-trip; injection is deterministic prompt-prefix insertion. Determinism
  extends to the chosen top-3 ordering: ties on match score MUST be broken by a
  stable secondary key (Architect pins; suggested baseline: descending
  `success_count - failure_count`, then ascending `playbook_id`).
- **NFR-3.3 (injection size budget)**: Injected playbook payload has a maximum byte
  budget over the aggregate top-3 payload. Oversize content is truncated with a
  trailing marker, and the truncation MUST emit
  `Misc{kind:"playbook_injection_truncated", original_bytes, capped_bytes,
  matched_count}` (matched_count <= 3).
- **NFR-3.4 (extraction input cap)**: Extraction LLM input (prompt + task context)
  must be capped at a fixed token budget. Over-budget context is truncated with an
  explicit `[..., truncated for extraction budget]` marker, and the truncation MUST
  emit `Misc{kind:"playbook_extraction_input_truncated", original_tokens,
  capped_tokens}`.
- **NFR-3.5 (extraction output cap)**: Extraction output must be capped at a fixed
  byte budget. Post-cap output must still satisfy F-3.18 minimum quality constraints.
  Cap hits MUST emit `Misc{kind:"playbook_extraction_output_capped", original_bytes,
  capped_bytes}` so F-3.18 floor failures can be traced back to truncation.
- **NFR-3.6 (search operability)**: Session search indexing must support all 8 event
  types and remain queryable from CLI/API without requiring full event-stream replay.

## 3. Functional requirements

- **F-3.1 (conservative trigger, content-agnostic)**: Extraction eligibility follows
  ADR-007 criteria and must remain content-agnostic. In Phase 3, extraction MUST fire
  for any task with verifier `pass` + `tool_calls >= 5`, regardless of
  `deliverable_format`, `project_id`, or title. ADR-007 criterion 3 (≥2 similar past
  tasks) and criterion 4 (optional user-satisfaction signal) are explicitly deferred
  to Phase 4 Curator and MUST NOT be implemented as Phase 3 trigger gates.
- **F-3.2 (Phase 3 acceptance benchmark fixture)**: Phase acceptance benchmark uses the
  deterministic `phase2_overnight_default_path`-style employee workflow shape.
- **F-3.3 (acceptance gate assertion)**: `cargo test phase3_warm_benchmark` must
  assert `sessions.tool_calls <= 0.70 x cold_baseline`, where cold baseline is the
  benchmark's tool-call count at Phase 3 kickoff lineage (around `cc7d4f0`).
- **F-3.4 (second-run identity for gate mode)**: Gate-mode “second run” is defined as
  same fixture ID + same normalized brief text, where "normalized" is the
  deterministic text-normalization step the gate matcher applies before comparison
  (Architect pins the exact rules; baseline: NFD Unicode + lowercase + collapse
  whitespace runs + strip leading/trailing whitespace). The same normalization
  function feeds both the gate matcher and any equality assertions in
  `phase3_warm_benchmark`.
- **F-3.5 (two matchers)**: Phase 3 ships two matchers:
  - Gate-mode strict identity over normalized brief (for deterministic benchmark).
  - Production-mode FTS5 prefix match over
    `playbooks.trigger_keywords ∪ title ∪ content`, which also un-stubs the Phase 0
    `playbook_search` tool so LLM-callable lookup returns real results.
  Matcher choice is runtime config; both emit the same `Skill` event shape (see
  F-3.8).
- **F-3.6 (tool-call canonical source)**: Acceptance gate uses `sessions.tool_calls` as
  canonical KPI counter. An adjacent regression test
  `sessions_tool_calls_matches_action_count` validates counter wiring integrity against
  the cold baseline; this parity test is not part of warm-gate success criteria.
- **F-3.7 (sync extraction execution model)**: Extraction runs synchronously in
  `task_complete` handling for Phase 3. Async workerization (Verifier-style
  XREADGROUP/FIFO/semaphore pattern) is Phase 4 Curator scope. Any non-timeout
  extraction failure (LLM call error, slot unavailable, malformed structured output)
  MUST emit `Misc{kind:"playbook_extraction_error", session_id, stage, reason}` where
  `stage ∈ {"prepare_input", "llm_call", "parse_output", "write_db"}`, and skip
  playbook write without blocking task completion.
- **F-3.8 (telemetry + counters)**: Phase 3 emits `Skill` EventType events with three
  sub-kinds — match (at any matcher hit, gate-mode or production-mode per F-3.5),
  injection (at top-3 injection), outcome (at task completion verifier verdict) —
  and maintains per-row
  `playbooks.success_count` / `failure_count` updated at task completion: verifier
  verdict `pass` increments `success_count`, verdict `fail` increments
  `failure_count`, and other verdicts (`error`, `skipped`, etc.) update neither
  counter and emit no outcome `Skill` event. Architect pins the exact `Skill` payload
  shape (e.g. `playbook_id`, `match_score`, `outcome`). Safety / observability emits
  in F-3.13/F-3.14/F-3.18 and NFR-3.1/3.3/3.4/3.5/F-3.7 use the `Misc` EventType,
  not `Skill`.
- **F-3.9 (no curator decisions in Phase 3)**: Phase 3 does not auto-archive,
  consolidate, or score-threshold playbooks. Those decisions remain Phase 4 Curator
  scope.
- **F-3.10 (SOP minimum surface, required)**: Phase 3 ships V010 `sops` table,
  real `sop_read` implementation (un-stubbing the Phase 0 placeholder, which remains
  the only LLM-callable SOP surface), and required CLI authoring surface:
  `seasoned-hand sop create/edit/list/show/delete` (CLI-only operator authoring;
  `show` mirrors F-3.20 playbook `show`). The agent does not gain a `sop_write`
  tool in Phase 3.
- **F-3.11 (top-3 injection)**: Phase 3 injects up to 3 matched playbooks at task
  start, ranked by match score, into Initializer system context. Zero matches is a
  valid result and skips injection silently (no `Misc` emit required); 1-2 matches
  inject those rows only.
- **F-3.12 (project-scoped matching in Phase 3)**: Matching is project-scoped
  (`source_task.project_id == new_task.project_id`, where `source_task` is the task
  from which the candidate playbook was extracted). `tenant_id` is not consulted for
  Phase 3 matching logic.
- **F-3.13 (layered adversarial filtering)**: Extraction applies two defense layers:
  - LLM prompt-level refusal guidance.
  - Deterministic post-extraction scan.
  Deterministic baseline MUST detect at minimum:
  1. shell substitution/metachar patterns (backticks, `$()`, pipe-to-shell `| sh`/`| bash`),
  2. raw IPv4/IPv6 literal hosts in URLs,
  3. prompt-injection trigger phrases (Architect-curated list including “ignore previous instructions”, “you are now”, role-reversal patterns),
  4. base64-shaped blobs of length >=40.
  Rejection emits `Misc{kind:"playbook_extraction_rejected", layer, reason}` where
  `layer ∈ {"llm", "deterministic", "quality_floor"}` is the discriminant shared with
  F-3.18 (consumers route on `layer`, not on a per-cause `kind`).
- **F-3.14 (layered PII/content redaction)**: Extraction applies two PII defenses:
  - LLM abstraction instruction (generalize concrete identifiers).
  - Deterministic regex redaction baseline MUST strip at minimum:
    1. high-entropy token-shaped strings (`[A-Za-z0-9_-]{32,}`),
    2. email address shapes,
    3. phone number shapes (E.164 + common locale formats),
    4. IPv4/IPv6 literals,
    5. bearer/API-key-like header patterns.
  Every redaction occurrence emits `Misc{kind:"playbook_extraction_pii_redacted",
  layer, count, categories}` for observability (symmetric with F-3.13 rejection
  event), where `layer ∈ {"llm", "deterministic"}` (the `quality_floor` value from
  F-3.13 does not apply to redaction). The `playbook_extraction_*` prefix is
  shared by all extraction-pipeline events (F-3.7/F-3.13/F-3.18 + NFR-3.1/3.4/3.5)
  so operators can grep them as a set.
- **F-3.15 (activation policy)**: Auto-extracted playbooks are immediately injectable
  (no quarantine state machine in Phase 3), consistent with ADR-007 Alternative C
  rejection and Phase 3 scope boundaries.
- **F-3.16 (session search index scope)**: Phase 3 ships FTS5-backed denormalized
  session search index covering all 8 event types:
  `Message, Action, Observation, Plan, Knowledge, Datasource, Skill, Misc`. The
  index schema is intentionally wider than Phase 3 emit: only 6 of 8 variants
  (`Message, Action, Observation, Plan, Skill, Misc`) are actively written in
  Phase 3; `Knowledge` and `Datasource` writers ship in Phase 4+ per §5 without
  requiring a search-index migration.
- **F-3.17 (session search summarization + CLI)**: Session search results include an
  LLM summarization path for operator consumption (query-centric summary over matched
  rows). Phase 3 ships an operator CLI surface (`seasoned-hand session search
  <query>`) that returns both raw matches and the summarized view, satisfying
  NFR-3.6's CLI/API queryability requirement; programmatic Rust callers consume the
  same internal API.
- **F-3.18 (minimum extraction quality bar)**: A playbook draft must satisfy required
  structural fields and a minimum non-trivial procedure body before write. Baseline
  floors that MUST hold unless Architect raises them after dogfood data: at least 3
  non-trivial procedure steps and at least 200 total characters of procedure body
  content (across whatever V010 field name Q1 settles on, e.g. `procedure_steps`
  or `content`). Drafts below either floor MUST be rejected with
  `Misc{kind:"playbook_extraction_rejected", layer:"quality_floor", reason}` per the
  F-3.13 `layer` vocabulary.
- **F-3.19 (atomic migration + spec reconciliation)**: V010 migration and required
  architecture-spec reconciliation land in the same PR slice per AGENTS.md §8. If
  immutable architecture text requires change, include successor ADR in the same slice.
  Intentional phased doc/schema drift windows (e.g. "land code now, fix spec later")
  are explicitly disallowed in Phase 3.
- **F-3.20 (playbook lifecycle CLI, required)**: Phase 3 ships required CLI lifecycle:
  `seasoned-hand playbook list/show/delete`.
- **F-3.21 (glossary minimum surface, required)**: Phase 3 lands the V010 `glossary`
  table per ARCH §2.5 (id/term/definition/category/timestamps) and un-stubs the
  Phase 0 `glossary_lookup` tool so it returns real rows. CLI authoring is deferred
  (operator may seed terms via SQL or future Phase 4 surface); the un-stubbed tool
  closes the Phase 1 `glossary_lookup` "Deferred to Phase 3+" marker.

## 4. Story breakdown

Each story is 1-3 hours, independently mergeable, and executable from fresh context.
Detailed contracts live in `/specs/phase-3/stories/story-3.X.md`.

| ID | Story title | Est | Deps | Status |
|---|---|---|---|---|
| 3.1 | Phase 3 scaffolds (requirements + architecture + DEBT + story map) | 0.5h | — | done |
| 3.2 | Atomic slice: V010 + ADR-012 + ARCH v1.2 + FTS5 triggers + tool un-stubs | 3h | 3.1 | done |
| 3.3 | Sync extraction orchestration in task-complete path | 2h | 3.2 | done |
| 3.4 | Extraction safety filters, redaction, quality floor, and cap events | 3h | 3.3 | done |
| 3.5 | Matcher core (gate identity + production FTS5 + deterministic ranking) | 3h | 3.2 | done |
| 3.6 | Initializer top-3 deterministic playbook injection | 2h | 3.5 | done |
| 3.7 | Skill telemetry outcomes and playbook counters | 2h | 3.6 | done |
| 3.8 | Session search index ingestion and internal query API | 2h | 3.2 | done |
| 3.9 | CLI SOP lifecycle surface | 2h | 3.2 | done |
| 3.10 | CLI playbook lifecycle surface | 1.5h | 3.2, 3.5 | done |
| 3.11 | CLI session search with summarized operator view | 2h | 3.8 | ready |
| 3.12 | Regression: sessions tool-call parity guard | 1.5h | 3.1 | ready |
| 3.13 | Regression: production matcher smoke | 1.5h | 3.5 | ready |
| 3.14 | Regression: FTS5 maintenance trigger correctness | 1h | 3.2, 3.8 | ready |
| 3.15 | Benchmark fixture + gate-mode harness | 2h | 3.2, 3.5, 3.6 | ready |
| 3.16 | Phase 3 acceptance gate and close-out | 2h | 3.12, 3.13, 3.14, 3.15 | ready |

Total: 16 stories, ~30 hours.

## 5. Acceptance criteria (Phase-level)

- `cargo test phase3_warm_benchmark` passes with
  `sessions.tool_calls <= 0.70 x cold_baseline` for the deterministic
  `phase2_overnight_default_path`-style fixture.
- Gate uses strict second-run identity: same fixture ID + normalized brief text.
- Gate metric source is `sessions.tool_calls`.
- Counter trust precondition is enforced by separate regression
  `sessions_tool_calls_matches_action_count`.
- Production-mode FTS5 matcher correctness is enforced by a separate regression
  (`phase3_production_matcher_smoke` or equivalent name) that seeds known
  playbook rows and asserts the FTS5 matcher returns the expected top-3 set on
  representative queries. Without this, `phase3_warm_benchmark` only exercises
  the gate matcher and the production matcher could ship broken.
- Benchmark gate remains deterministic and CI-runnable without manual evaluation.

## 6. Out of scope (explicitly deferred)

- Diversity validation across additional task families beyond the benchmark fixture.
  (Phase 4 Curator scope.)
- Async extraction workerization and queue-ops tuning. (Phase 4 Curator scope.)
- Playbook auto-archive, duplicate consolidation, and automated quality decisions.
  (Phase 4 Curator scope.)
- Frontend SOP editor. (Phase 5 multi-user scope.)
- Tenant isolation semantics (`NULL`-as-global, cross-tenant promotion tooling,
  admin policy surfaces). (Phase 5 multi-user scope.)
- Playbook export/sharing workflows. (Phase 4+ once sharing semantics are defined.)
- Quarantine/pending activation workflows for playbooks. (Phase 5 scope.)
- ADR-007 criterion 3 (≥2 similar past tasks) and criterion 4 (optional
  user-satisfaction signal). (Phase 4 Curator scope.)
- L2 cross-source verification (ARCH §6 layer 2). Phase 3 does not implement L2
  enforcement; the layer ships in Phase 4 alongside `Knowledge` event writers.
- Active `Knowledge` and `Datasource` event emit. Both EventType variants stay
  reserved-but-unwired in Phase 3; writers ship in Phase 4+ (paired with L2 above).
  Phase 2 DEBT #61 stays partially open against these two variants.
- Glossary CLI authoring surface (operator seeds terms via SQL in Phase 3); CLI
  surface deferred to Phase 4+ once usage patterns settle.

## 7. Risks and mitigations

- **Risk: benchmark overfitting**
  - Mitigation: F-3.1 content-agnostic extraction predicate and production matcher
    decoupled from gate matcher (F-3.5); diversity validation explicitly carried to
    Phase 4.
- **Risk: poisoned extraction from adversarial observations**
  - Mitigation: layered defenses in Phase 3 — verifier-PASS gate (F-3.1 / ADR-007
    criterion 1) + LLM filtering and deterministic scan (F-3.13) + project-scoped
    matching (F-3.12).
- **Risk: PII leakage into reusable artifacts**
  - Mitigation: layered abstraction+redaction (F-3.14), project-scoped reuse (F-3.12),
    operator delete hatch (F-3.20).
- **Risk: sync extraction latency impacts completion UX**
  - Mitigation: hard 60s timeout + non-blocking completion semantics (NFR-3.1).
- **Risk: playbook bloat drives prompt cost and instability**
  - Mitigation: the NFR-3.3/3.4/3.5 caps (injection, extraction input, extraction
    output) plus the F-3.18 minimum-quality guard.
- **Risk: counter drift undermines acceptance metric**
  - Mitigation: explicit canonical metric source (F-3.6) + parity regression test.
- **Risk: systemic extraction failure (LLM gateway down, slot unavailable across
  the dogfood window) leaves the playbook table empty and silently fails the F-3.3
  acceptance gate for non-Phase-3 reasons**
  - Mitigation: F-3.7 `playbook_extraction_error` event is auditable from the event
    stream; CI MUST treat repeated extraction errors across the benchmark fixture
    as a gate-blocking infrastructure failure distinct from a learning regression.

## 8. Dependencies (external + internal)

- **Internal spec/process dependencies**
  - AGENTS.md §8 (same-PR spec/code reconciliation discipline)
  - ADR-007 (conservative learning gate)
  - ROADMAP Phase 3/4/5 boundaries
- **Internal schema/runtime dependencies**
  - V010 migration must include artifacts required by F-3.8 / F-3.10 / F-3.21
    (`success_count`, `failure_count`, `sops`, `glossary`) plus the playbook fields
    needed by F-3.5/F-3.11 (`trigger_keywords`, etc.) and a session search index
    (e.g. `events_fts`) covering all 8 event types per F-3.16
  - Initializer prompt path for top-3 injection
  - Event stream emit surfaces for `Skill` learning events (F-3.8) AND `Misc`
    operational/safety events (NFR-3.1/3.3/3.4/3.5, F-3.7/3.13/3.14/3.18) plus
    search-index ingestion (F-3.16/3.17)
- **Operational dependencies**
  - Deterministic benchmark harness (`phase3_warm_benchmark`) plus the production
    matcher regression added by §4
  - CLI surfaces: SOP lifecycle (F-3.10), playbook lifecycle (F-3.20), and session
    search (F-3.17)

## 9. Open questions

- Exact regex strings and threshold tuning for deterministic safety scans/redaction
  (false-positive vs false-negative balance).
- Production matcher scoring details: FTS5 weighting, score threshold, and
  tie-breaking when more than 3 rows match (F-3.11 caps injection at 3).
- Exact token/byte budgets for NFR-3.3/3.4/3.5 and coupling to model context windows.
- Whether F-3.18 baseline floors (3 steps / 200 chars) need raising after dogfood
  data, and how to define "non-trivial" for the step-count check.
- Session-search summarization model-slot specifics and result-ranking strategy.
- Schema-shape finalization details for V010 fields beyond the hard constraints in
  this doc (while still satisfying F-3.5/F-3.8/F-3.10/F-3.16/F-3.21 — playbook
  fields, counters, sops, events_fts, glossary).
