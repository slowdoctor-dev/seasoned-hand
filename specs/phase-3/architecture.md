# Phase 3 — Architecture

> Status: v1.0 (BMAD Architect)
> Date: 2026-05-17
> Inputs: `/specs/phase-3/requirements.md` (contract), `/specs/phase-3/OPEN_QUESTIONS.md`
> Immutable base: `/specs/01-architecture/ARCHITECTURE.md` v1.1 + ADR-007/010/011

## 1. Summary diagram

```text
Task complete (verifier PASS + tool_calls>=5)
        |
        v
[Sync Extraction Pipeline] --(guardrails/redaction/quality floor)--> [playbooks]
        |                                                           |
        +--> Misc(playbook_extraction_*) events                     +--> playbooks_fts

New task start
   |
   v
[Matcher]
  - gate mode: strict normalized-brief identity
  - prod mode: FTS5 prefix over keywords/title/content
   |
   v
Top-3 deterministic rank + byte cap
   |
   v
Initializer prompt-prefix injection (no extra LLM round-trip)
   |
   v
Skill events (match/injection), later outcome counters on completion

Operators
  - CLI: sop create/edit/list/show/delete
  - CLI: playbook list/show/delete
  - CLI: session search <query> (raw + summary)
  - Tool un-stubs: sop_read / playbook_search / glossary_lookup
```

## 2. New components introduced

1. `LearningExtractor` (core)
- Purpose: synchronous extraction on `task_complete` (F-3.7) with ADR-007 criteria 1+2 only.
- Technology: Rust async service in `seasoned-hand-core`. Extraction LLM call resolves via
  `SlotRouter::resolve(SlotName::Planner)` (precedent: `deliverable/task_deliver.rs:130`).
  Planner slot fits: extraction is structured-output drafting from verified work, the same
  shape as plan decomposition. Verifier slot is reserved for FAIL-biased L4 verification
  per ARCH §6 and is NOT reused here.
- Integration: task completion path, events writer (`Misc`), V010 playbook writes.

2. `PlaybookMatcher` (core)
- Purpose: runtime-selectable gate matcher + production matcher (F-3.5).
- Technology: deterministic text normalizer + SQLite FTS5 query builder.
- Integration: Initializer injection path, un-stubbed `playbook_search` tool, skill telemetry emit.

3. `PlaybookInjector` (core)
- Purpose: deterministic top-3 ranking, prompt-prefix insertion, NFR-3.3 cap handling.
- Technology: pure Rust ranking/truncation utility.
- Integration: `agent/init` system prompt assembly only (not per-iteration sticky insertion).

4. `SessionSearchIndex` (core/server)
- Purpose: denormalized FTS5 index over all 8 event types (F-3.16/3.17).
- Technology: SQLite table + FTS5 virtual table + ingestion hook on event append.
- Integration: CLI/API session search + summary route.

5. CLI command groups
- `seasoned-hand sop ...`
- `seasoned-hand playbook ...`
- `seasoned-hand session search <query>`

## 3. Data model changes

V010 is **hybrid schema reconciliation** (Q1 option C under F-constraints): keep V009 compatibility, add required rich fields.

```sql
-- existing playbooks table is extended
ALTER TABLE playbooks ADD COLUMN trigger_keywords TEXT NOT NULL DEFAULT '[]';
ALTER TABLE playbooks ADD COLUMN content TEXT NOT NULL DEFAULT '';
ALTER TABLE playbooks ADD COLUMN procedure_body TEXT NOT NULL DEFAULT '';
ALTER TABLE playbooks ADD COLUMN success_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE playbooks ADD COLUMN failure_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE playbooks ADD COLUMN avg_duration_ms INTEGER;
ALTER TABLE playbooks ADD COLUMN avg_tool_calls INTEGER;
ALTER TABLE playbooks ADD COLUMN status TEXT NOT NULL DEFAULT 'active';
ALTER TABLE playbooks ADD COLUMN version INTEGER NOT NULL DEFAULT 1;

-- V009 content_path retained for hybrid storage
-- content_path NULL => inline-only row
-- content_path non-NULL => spilled full body file path (content/procedure_body keep searchable excerpt)

CREATE VIRTUAL TABLE playbooks_fts USING fts5(
  title, trigger_keywords, content,
  content='playbooks',
  content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2'
);

CREATE TABLE sops (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  content TEXT NOT NULL,
  version INTEGER NOT NULL,
  enforced BOOLEAN NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE glossary (
  id TEXT PRIMARY KEY,
  term TEXT NOT NULL UNIQUE,
  definition TEXT NOT NULL,
  category TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE session_search_index (
  event_id INTEGER PRIMARY KEY,
  session_id TEXT NOT NULL,
  timestamp INTEGER NOT NULL,
  event_type TEXT NOT NULL,
  source TEXT NOT NULL,
  searchable_text TEXT NOT NULL
);

CREATE VIRTUAL TABLE session_search_fts USING fts5(
  searchable_text,
  content='session_search_index',
  content_rowid='event_id',
  tokenize='unicode61 remove_diacritics 2'
);
```

Pinned NFR budgets:
- NFR-3.3 injection cap: `12_288 bytes` aggregate across injected top-3 payload.
- NFR-3.4 extraction input cap: `8_192 tokens` (prompt + context).
- NFR-3.5 extraction output cap: `24_576 bytes`.

Quality-floor field (F-3.18): `procedure_body`.
- Must contain >=3 non-trivial ordered steps.
- Must contain >=200 UTF-8 characters after deterministic redaction/capping.

## 4. API surface

Internal/core APIs:
- `extract_playbook_sync(task_id, session_id) -> Result<Option<PlaybookDraft>>`
- `match_playbooks(project_id, brief, mode) -> Vec<MatchedPlaybook>`
- `inject_playbooks(system_prefix, matches) -> InjectedBlock`
- `record_playbook_outcome(task_id, verifier_verdict)`
- `index_event_for_search(event)` and `search_session_events(query, filters, limit)`

Tool un-stubs:
- `sop_read`: returns concrete SOP rows (title/content/version/enforced).
- `playbook_search`: executes production matcher and returns ranked rows.
- `glossary_lookup`: exact/FTS-backed lookup over glossary terms.

CLI required surfaces:
- `seasoned-hand sop create|edit|list|show|delete`
- `seasoned-hand playbook list|show|delete`
- `seasoned-hand session search <query>` (raw hits + summarized view)

Event payload shapes:

`Skill` events — F-3.8 sub-kinds (no `playbook_` prefix; that prefix is reserved for
extraction-pipeline `Misc` events per F-3.14):
- `Skill{kind:"match", playbook_id, project_id, matcher_mode, match_score}` —
  emitted per matcher hit in either gate or production mode (F-3.5/F-3.8).
- `Skill{kind:"injection", injected_ids:[...], total_bytes, truncated}` —
  emitted once per task at top-3 injection (F-3.11).
- `Skill{kind:"outcome", playbook_id, outcome:"pass"|"fail"}` —
  emitted only when verifier verdict is `pass` or `fail` (F-3.8). `error`/`skipped`
  verdicts emit NO outcome event and update neither counter; the same handler that
  emits the outcome event increments `success_count` (pass) or `failure_count` (fail).

Extraction-pipeline `Misc` events — share `playbook_extraction_*` prefix per F-3.14,
so operators can grep them as a set:
- `playbook_extraction_error{session_id, stage, reason}` —
  `stage ∈ {"prepare_input","llm_call","parse_output","write_db"}` per F-3.7.
- `playbook_extraction_timeout{session_id, elapsed_ms}` (NFR-3.1).
- `playbook_extraction_input_truncated{original_tokens, capped_tokens}` (NFR-3.4).
- `playbook_extraction_output_capped{original_bytes, capped_bytes}` (NFR-3.5).
- `playbook_extraction_rejected{layer, reason}` —
  `layer ∈ {"llm","deterministic","quality_floor"}` per F-3.13/F-3.18. Quality-floor
  failures (F-3.18) reuse this kind with `layer="quality_floor"`, NOT a separate kind.
- `playbook_extraction_pii_redacted{layer, count, categories}` —
  `layer ∈ {"llm","deterministic"}` per F-3.14 (the `quality_floor` value does not
  apply to redaction).

Injection-cap `Misc` event (NFR-3.3) — sits under the same operator grep set but is
emitted from the injection path, not the extraction pipeline:
- `playbook_injection_truncated{original_bytes, capped_bytes, matched_count}` —
  `matched_count <= 3`.

## 5. External dependencies

No new platform components beyond immutable stack.
- DB/search remains SQLite + FTS5.
- Routing remains existing Bifrost + `SlotRouter`.
- No new service dependency required by architecture.

## 6. Interactions with existing components

- `task_complete` handling adds synchronous extraction call with 60s timeout.
- Initializer (`agent/init`) gains one-shot playbook prompt-prefix injection.
- Event append path now also writes denormalized search row.
- Verifier verdict path updates playbook counters and emits `Skill{kind:"outcome"}` ONLY
  for `pass` / `fail` verdicts (F-3.8). `error` / `skipped` / other verdicts update no
  counter and emit no outcome event.
- Existing `EventType` taxonomy remains; Phase 3 emits `Skill`, leaves `Knowledge`/`Datasource` writers deferred.
- Phase 0 deferred tools (`sop_read`, `playbook_search`, `glossary_lookup`) are made real.

## 7. Performance budget

- Extraction pipeline end-to-end timeout: `<= 60_000 ms` hard stop.
- Extraction prep + truncation: `<= 500 ms` p95 (excluding LLM call).
- Matcher query p95 (project-scoped, <=10k playbooks): `<= 80 ms`.
- Injection assembly p95: `<= 20 ms`.
- Session search query p95 (single session, <=50k indexed events): `<= 120 ms`.
- Search summarization adds one auxiliary LLM call via `SlotRouter::resolve(SlotName::SessionSearch)`
  (the existing `session_search` aux slot per ARCH §3.2). Failures degrade to raw hits only.

## 8. Failure modes

- LLM extraction call fails/slot unavailable/malformed output:
  - Emit `Misc{kind:"playbook_extraction_error", stage, reason}`.
  - Skip write; do not block completion.
- Extraction exceeds 60s:
  - Emit `playbook_extraction_timeout`; skip write.
- Input/output budget exceeded:
  - Deterministic truncation/cap + corresponding `playbook_extraction_*` `Misc` event.
- Adversarial scan or quality floor reject:
  - Emit `playbook_extraction_rejected{layer,reason}`; skip write.
- PII detected:
  - Redact + emit `playbook_extraction_pii_redacted{layer,count,categories}`.
- Search summarizer LLM fails:
  - Return raw FTS hits; emit warning `Misc`.

## 9. Security considerations

Layered extraction safety per F-3.13/F-3.14 — Phase 3 ships BOTH layers; the LLM layer
is probabilistic and the deterministic layer is the audit floor.

Deterministic adversarial scan baseline (F-3.13 — MUST detect, at minimum):
1. shell substitution / metacharacters: backticks, `$(...)`, pipe-to-shell `| sh` / `| bash`
2. raw IPv4 / IPv6 literal hosts in URLs
3. prompt-injection trigger phrases (Architect-curated list): "ignore previous instructions",
   "you are now", role-reversal patterns (e.g. "you are not the assistant")
4. base64-shaped blobs of length >=40 (`[A-Za-z0-9+/=]{40,}`)

Deterministic PII redaction baseline (F-3.14 — MUST strip, at minimum):
1. high-entropy token-shaped strings: `[A-Za-z0-9_-]{32,}`
2. email address shapes
3. phone number shapes: E.164 plus common locale formats
4. IPv4 / IPv6 literals
5. bearer / API-key-like header patterns (e.g. `Authorization: Bearer ...`, `X-Api-Key: ...`)

Layer-discriminant vocabulary (shared across F-3.13 / F-3.14 / F-3.18 — see §4 for emit
shape):
- `playbook_extraction_rejected.layer ∈ {"llm","deterministic","quality_floor"}`. F-3.18
  quality-floor failures reuse this kind with `layer="quality_floor"`, NOT a separate kind.
- `playbook_extraction_pii_redacted.layer ∈ {"llm","deterministic"}` (`quality_floor`
  does not apply to redaction).

Other Phase 3 security controls:
- Project-scoped matching is mandatory (`source_task.project_id == target.project_id`, F-3.12);
  `tenant_id` is not consulted for Phase 3 matching logic.
- No auto-archive / quarantine policy in Phase 3 (F-3.15); operator delete via
  `seasoned-hand playbook delete <id>` (F-3.20) is the required escape hatch.

## 10. Migration plan

F-3.19 atomic slice rule — V010 + spec reconciliation + any required ADR land in the
SAME PR slice. Intentional doc/schema drift windows are explicitly disallowed in Phase 3.

1. Land V010 migration (extended `playbooks` columns + `playbooks_fts` + `sops` +
   `glossary` + `session_search_index` + `session_search_fts`).
2. Un-stub `sop_read`, `playbook_search`, `glossary_lookup` in same slice. Tool catalog
   count stays at 38 unique (39 `map.insert` entries per `spec-check.sh`) — un-stubbing
   replaces stub bodies, no new tools are registered.
3. Reconcile architecture spec in same slice. **The hybrid storage choice (retaining
   `content_path` from V009 alongside the new inline `content` column) IS a divergence
   from ARCH §2.5, which spec's `content TEXT NOT NULL` only — no `content_path`.** Per
   F-3.19, this divergence requires a successor ADR (ADR-012, sibling pattern to
   ADR-011's v1.0→v1.1 reconciliation) in the SAME PR slice, bumping ARCH §2.5 to v1.2
   to acknowledge the inline + optional-spill hybrid. The same ADR formalizes the V010
   ALTER-TABLE shape (DEFAULT-backfilled NOT NULL columns) as the canonical schema.
4. Backfill existing V009 rows:
   - `trigger_keywords='[]'`, `content=''`, `procedure_body=''`, `status='active'`,
     `version=1`.
   - Keep `content_path` as-is; extractor writes inline by default and MAY spill the
     full body to `content_path` when extraction output exceeds the 16_384-byte spill
     threshold (bounded by NFR-3.5's 24_576-byte hard cap on extraction output).
5. New tables (`sops`, `glossary`, `session_search_index`, `session_search_fts`) start
   empty at V010; no backfill required.
6. Update `scripts/spec-check.sh` (closes Phase 2 DEBT #62 carry-forward) to enforce:
   V010 migration file present, `sops` + `glossary` table references, CLI command
   groups (`sop`, `playbook`, `session search`) registered. ARCH version bump v1.1→v1.2
   gates a Phase 3-version check.

## 11. Testing strategy

- Unit
  - deterministic normalizer (NFD + lowercase + whitespace collapse + trim)
  - tie-break ordering function
  - adversarial scan/redaction matchers
  - quality-floor validator on `procedure_body`
- Integration
  - sync extraction success/failure/timeout paths
  - production matcher FTS5 smoke (`phase3_production_matcher_smoke`)
  - tool un-stubs return real rows
  - session index ingestion + queryability across all 8 event types
- Acceptance (Phase 3 gate — `cargo test phase3_warm_benchmark`)
  - `phase3_warm_benchmark` enforces `sessions.tool_calls <= 0.70 x cold_baseline`
    (F-3.3 / requirements §4).
- Regression guards (separate from acceptance gate per F-3.6)
  - `sessions_tool_calls_matches_action_count` — parity test validating
    `sessions.tool_calls` wiring integrity against the cold baseline. NOT part of
    warm-gate success criteria (F-3.6); a counter-drift failure here fails CI but
    distinguishes "metric trust broken" from "learning regression".
  - `phase3_production_matcher_smoke` — seeds known playbook rows and asserts the
    FTS5 production matcher returns the expected top-3 on representative queries
    (requirements §4). Without this, `phase3_warm_benchmark` only exercises the gate
    matcher and the production matcher could ship broken.

Deterministic ranking/tie-break (NFR-3.2):
1. Primary: `match_score DESC`
2. Secondary: `(success_count - failure_count) DESC`
3. Tertiary: `success_count DESC`
4. Quaternary: `playbook_id ASC` (stable deterministic final key)

FTS5 scoring details (production matcher, F-3.5):
- Query: prefix tokens (`token*`) over union of `trigger_keywords ∪ title ∪ content`.
- Rank function:
  - `score = 5.0*kw_hits + 3.0*title_hits + 1.0*content_hits + recency_boost`
  - `recency_boost = max(0.0, 0.5 - (days_since_now(created_at) / 60.0))` —
    linear decay from +0.5 for a freshly created playbook to 0.0 once 30 days old.
    Recency matters most in early Phase 3 dogfood when a small population of playbooks
    competes for the same top-3 slot. (Note: Codex's initial formula
    `min(0.5, ln(1+days_since_epoch(created_at))/100)` was a math bug — for any modern
    date `days_since_epoch ≈ 20_000`, so `ln(1+20000)/100 ≈ 0.099` is effectively a
    constant added to every score and provides no rank signal. Fixed here.)
- Minimum acceptance threshold: `score >= 1.0`.
- Weights (`5.0` / `3.0` / `1.0`) and the 60-day decay constant are seed values; Phase 4
  Curator may retune from `Skill{kind:"match"}` telemetry. See DEBT seed #76.
- Above-threshold candidates sorted by the deterministic ranking pipeline above; inject
  up to top 3 (F-3.11).

## 12. Open technical questions

Decisions intentionally deferred (seeded to DEBT / Phase 4+):
1. Embedding-based semantic reranking over FTS shortlist (requires active embedding slot wiring).
2. Curator policies for archive/consolidation/rate thresholds using Phase 3 counters.
3. Knowledge/Datasource writer semantics and L2 cross-source enforcement rollout.
4. Tenant/global playbook semantics (Phase 5 multi-user policy).

Design alternatives considered and chosen:
1. V010 schema shape
- Option A rich (full ARCH): single-step completeness, more dead fields early.
- Option B minimal (V009-like): lighter now, conflicts with F-3.8/F-3.10/F-3.21.
- Option C hybrid (chosen): add required rich fields now, retain `content_path` compatibility.
2. Content storage
- Inline only: simplest read/search, row bloat risk.
- Path only: compact rows, indexing complexity.
- Hybrid (chosen): inline searchable body + optional spill path for large full bodies.
3. Injection site
- Initializer one-shot (chosen): zero extra per-iteration cost, matches F-3.11/NFR-3.2.
- Sticky every iteration: better persistence but ongoing token tax.
