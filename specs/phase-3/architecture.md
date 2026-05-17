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
- Technology: Rust async service in `seasoned-hand-core`, planner/verifier slot-style LLM call plumbing via existing `SlotRouter`.
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
- `Skill{kind:"playbook_match", playbook_id, project_id, matcher_mode, match_score}`
- `Skill{kind:"playbook_injection", injected_ids:[...], total_bytes, truncated}`
- `Skill{kind:"playbook_outcome", playbook_id, outcome:"pass"|"fail"}`
- Extraction pipeline `Misc` events share `playbook_extraction_*` prefix:
  - `playbook_extraction_error`
  - `playbook_extraction_timeout`
  - `playbook_extraction_input_truncated`
  - `playbook_extraction_output_capped`
  - `playbook_extraction_rejected`
  - `playbook_extraction_pii_redacted`
  - plus injection cap event `playbook_injection_truncated`

## 5. External dependencies

No new platform components beyond immutable stack.
- DB/search remains SQLite + FTS5.
- Routing remains existing Bifrost + `SlotRouter`.
- No new service dependency required by architecture.

## 6. Interactions with existing components

- `task_complete` handling adds synchronous extraction call with 60s timeout.
- Initializer (`agent/init`) gains one-shot playbook prompt-prefix injection.
- Event append path now also writes denormalized search row.
- Verifier verdict path updates playbook counters and emits `Skill` outcome.
- Existing `EventType` taxonomy remains; Phase 3 emits `Skill`, leaves `Knowledge`/`Datasource` writers deferred.
- Phase 0 deferred tools (`sop_read`, `playbook_search`, `glossary_lookup`) are made real.

## 7. Performance budget

- Extraction pipeline end-to-end timeout: `<= 60_000 ms` hard stop.
- Extraction prep + truncation: `<= 500 ms` p95 (excluding LLM call).
- Matcher query p95 (project-scoped, <=10k playbooks): `<= 80 ms`.
- Injection assembly p95: `<= 20 ms`.
- Session search query p95 (single session, <=50k indexed events): `<= 120 ms`.
- Search summarization adds one auxiliary LLM call; failures degrade to raw hits only.

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

- Layered extraction safety per F-3.13/F-3.14:
  - LLM refusal guidance then deterministic scan/redaction.
- Deterministic pattern checks include shell metachar injections, raw IP literals, role-reversal prompt-injection phrases, base64-like blobs >=40 chars.
- Project-scoped matching is mandatory in Phase 3 (`source_task.project_id == target.project_id`).
- No auto-archive/quarantine policy in Phase 3; operator delete remains required control.

## 10. Migration plan

F-3.19 atomic slice rule:
1. Land V010 migration (schema + FTS + sops/glossary).
2. Un-stub `sop_read`, `playbook_search`, `glossary_lookup` in same slice.
3. Reconcile architecture spec in same slice (this file; immutable ARCH ADR only if touched).
4. Backfill existing V009 rows:
- `trigger_keywords='[]'`, `content=''`, `procedure_body=''`, `status='active'`, `version=1`.
- Keep `content_path` as-is; extractor writes inline by default and may spill full body when output exceeds `16_384 bytes`.

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
- Acceptance
  - `phase3_warm_benchmark` enforces `sessions.tool_calls <= 0.70 x cold_baseline`
  - `sessions_tool_calls_matches_action_count` parity guard

Deterministic ranking/tie-break (NFR-3.2):
1. Primary: `match_score DESC`
2. Secondary: `(success_count - failure_count) DESC`
3. Tertiary: `success_count DESC`
4. Quaternary: `playbook_id ASC` (stable deterministic final key)

FTS5 scoring details (production matcher):
- Query: prefix tokens (`token*`) over union of `trigger_keywords`, `title`, `content`.
- Rank function:
  - `score = 5.0*kw_hits + 3.0*title_hits + 1.0*content_hits + recency_boost`
  - `recency_boost = min(0.5, ln(1 + days_since_epoch(created_at))/100)`
- Minimum acceptance threshold: `score >= 1.0`.
- Above-threshold candidates sorted by deterministic ranking pipeline above; inject up to top 3.

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
