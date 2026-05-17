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
Skill events (match / injection / outcome) + success_count / failure_count counters on completion

Operators
  - CLI: sop create / edit / list / show / delete
  - CLI: playbook list / show / delete (soft-delete via status='archived')
  - CLI: session search <query> (raw hits + LLM-summarized view)
  - Tool un-stubs: sop_read / playbook_search / glossary_lookup
  - Glossary CLI: deferred to Phase 4+ per F-3.21 (operator seeds via SQL in Phase 3)
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
- Technology: deterministic text normalizer + SQLite FTS5 query builder. The shared
  normalizer (used by BOTH gate and production matchers so a brief that hits in gate
  mode also hits in production mode) applies in this order per F-3.4: NFD Unicode
  normalization → ASCII lowercase → collapse runs of Unicode whitespace to a single
  space → strip leading/trailing whitespace. Production matcher issues FTS5 prefix
  tokens (`token*`) over the union of `playbooks.trigger_keywords ∪ title ∪ content`
  per F-3.5 (scoring details in §11).
- Gate-mode identity (F-3.4): match key is the tuple `(fixture_id, normalized_brief)`.
  Both halves must match exactly — fixture_id alone is not sufficient (rules out
  cross-brief reuse within the same fixture); normalized_brief alone is not sufficient
  (rules out cross-fixture coincidence). Production mode does not use fixture_id.
- Both matchers WHERE-filter `status = 'active'` so soft-deleted ('archived') playbooks
  never match (consistent across LLM-callable `playbook_search` tool and Initializer
  injection path).
- F-3.12 project-scope enforcement: matcher JOINs `playbooks` → `tasks` (via
  `playbooks.source_task_id`) and filters `tasks.project_id = :new_task.project_id`.
  No denormalized `source_project_id` column in V010 — the JOIN is cheap at Phase 3
  scale (<=10k playbooks per matcher query budget in §7). Denormalization is a Phase 4
  perf escape valve if the JOIN becomes a hot path (see DEBT seed #77).
- Integration: Initializer injection path, un-stubbed `playbook_search` tool, skill telemetry emit.

3. `PlaybookInjector` (core)
- Purpose: deterministic top-3 ranking, prompt-prefix insertion, NFR-3.3 cap handling.
- Technology: pure Rust ranking/truncation utility.
- Integration: `agent/init` system prompt assembly only (not per-iteration sticky insertion).
- Zero-match behavior (F-3.11): silent skip — no injection block, no `Misc` event, no
  `Skill{kind:"injection"}` event. 1-2 matches inject those rows only (no padding).

4. `SessionSearchIndex` (core/server)
- Purpose: denormalized FTS5 index over all 8 event types (F-3.16/3.17).
- Technology: SQLite table + FTS5 virtual table + ingestion hook on event append.
- Integration: CLI/API session search + summary route.

5. CLI command groups
- `seasoned-hand sop ...` (F-3.10 required surface)
- `seasoned-hand playbook ...` (F-3.20 required surface; `delete` is soft-delete —
  sets `status='archived'`, preserves the row for audit)
- `seasoned-hand session search <query>` (F-3.17 required surface)
- `seasoned-hand glossary ...` is intentionally NOT shipped in Phase 3 per F-3.21
  (operator seeds glossary terms via SQL; CLI authoring deferred to Phase 4+).

## 3. Data model changes

V010 is **hybrid schema reconciliation** (Q1 option C under F-constraints): keep V009
compatibility, add the F-3.5 / F-3.8 / F-3.10 / F-3.16 / F-3.21-required rich fields,
and use a single `content` column for the full playbook body (per F-3.18's allowance
to use `content` directly as the quality-floor target field).

```sql
-- existing playbooks table is extended
ALTER TABLE playbooks ADD COLUMN trigger_keywords TEXT NOT NULL DEFAULT '[]';
ALTER TABLE playbooks ADD COLUMN content TEXT NOT NULL DEFAULT '';
ALTER TABLE playbooks ADD COLUMN success_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE playbooks ADD COLUMN failure_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE playbooks ADD COLUMN avg_duration_ms INTEGER;
ALTER TABLE playbooks ADD COLUMN avg_tool_calls INTEGER;
ALTER TABLE playbooks ADD COLUMN status TEXT NOT NULL DEFAULT 'active';
-- status ∈ {'active', 'archived', 'pinned'} per ARCH §2.5. Phase 3 writes 'active'
-- on extraction and 'archived' via CLI soft-delete (F-3.20). 'pinned' is reserved
-- for Phase 4 Curator. CHECK constraint omitted to match ARCH §2.5's prose-only
-- spec; production matcher WHERE clause filters `status = 'active'`.
ALTER TABLE playbooks ADD COLUMN version INTEGER NOT NULL DEFAULT 1;

-- V009 content_path is retained as a reserved-for-Phase-4 column. Phase 3 writes
-- empty string '' (V009's NOT NULL constraint is inherited; no schema dance to drop
-- it). content_path stays unused in Phase 3 because NFR-3.5's 24_576-byte extraction
-- output cap is small enough to inline cleanly without spill. Phase 4 may activate
-- spill semantics when curation produces larger composed playbooks; that activation
-- pairs with the ADR-012 follow-up (see §10.3).

CREATE VIRTUAL TABLE playbooks_fts USING fts5(
  title, trigger_keywords, content,
  content='playbooks',
  content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2'
);

-- External-content FTS5 (per ARCH §2.5) does NOT auto-mirror playbooks updates;
-- V010 ships the three standard maintenance triggers so app code can issue plain
-- INSERT / UPDATE / DELETE against playbooks without manually touching the FTS index.
-- content_rowid='rowid' is SQLite's implicit row identifier (NOT playbooks.id, which
-- is a TEXT UUID); all FTS5 ↔ playbooks joins must use rowid, not id.
CREATE TRIGGER playbooks_ai AFTER INSERT ON playbooks BEGIN
  INSERT INTO playbooks_fts(rowid, title, trigger_keywords, content)
  VALUES (new.rowid, new.title, new.trigger_keywords, new.content);
END;
CREATE TRIGGER playbooks_ad AFTER DELETE ON playbooks BEGIN
  INSERT INTO playbooks_fts(playbooks_fts, rowid, title, trigger_keywords, content)
  VALUES ('delete', old.rowid, old.title, old.trigger_keywords, old.content);
END;
CREATE TRIGGER playbooks_au AFTER UPDATE ON playbooks BEGIN
  INSERT INTO playbooks_fts(playbooks_fts, rowid, title, trigger_keywords, content)
  VALUES ('delete', old.rowid, old.title, old.trigger_keywords, old.content);
  INSERT INTO playbooks_fts(rowid, title, trigger_keywords, content)
  VALUES (new.rowid, new.title, new.trigger_keywords, new.content);
END;

-- sops: no tenant_id column in Phase 3 (single-operator scope per Phase 3/5 boundary
-- in requirements.md §1). Phase 5 multi-user will add a nullable tenant_id with the
-- same NULL-then-NOT-NULL pattern V009 used for playbooks; deferred to ADR-013 at
-- Phase 5 kickoff.
CREATE TABLE sops (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  content TEXT NOT NULL,
  version INTEGER NOT NULL,
  enforced BOOLEAN NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
-- sop_read({title}) lookup path needs an index; PRIMARY KEY already covers id-lookup.
CREATE INDEX idx_sops_title ON sops(title);

-- glossary: no tenant_id in Phase 3 (same Phase 3/5 boundary reasoning as sops).
-- UNIQUE(term) is global (no per-tenant scoping); Phase 5 may relax to UNIQUE(tenant_id, term).
CREATE TABLE glossary (
  id TEXT PRIMARY KEY,
  term TEXT NOT NULL UNIQUE,
  definition TEXT NOT NULL,
  category TEXT NOT NULL,
  -- category ∈ {'person', 'system', 'terminology', 'context'} per ARCH §2.5
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

-- session_search_index is populated synchronously on event append; one row per event
-- across all 8 EventType variants per F-3.16 (Phase 3 actively writes 6 of 8;
-- Knowledge / Datasource writers ship in Phase 4+ without a schema migration).
-- event_type CHECK mirrors the events.type CHECK from ARCH §2.1 (V002) so a writer
-- that produces an unknown EventType cannot smuggle a row into the search index
-- ahead of the canonical events table.
CREATE TABLE session_search_index (
  event_id INTEGER PRIMARY KEY,
  session_id TEXT NOT NULL,
  timestamp INTEGER NOT NULL,
  event_type TEXT NOT NULL CHECK(event_type IN (
    'Message','Action','Observation','Plan',
    'Knowledge','Datasource','Skill','Misc'
  )),
  source TEXT NOT NULL,
  searchable_text TEXT NOT NULL
);
-- Lookup index for the common "filter by session, scan by time" CLI query.
CREATE INDEX idx_session_search_session_time
  ON session_search_index(session_id, timestamp);
-- Optional filter on event_type (CLI `--type=Action`).
CREATE INDEX idx_session_search_type
  ON session_search_index(event_type);

CREATE VIRTUAL TABLE session_search_fts USING fts5(
  searchable_text,
  content='session_search_index',
  content_rowid='event_id',
  tokenize='unicode61 remove_diacritics 2'
);

-- Same external-content FTS5 maintenance triggers for the session search index;
-- content_rowid='event_id' here matches session_search_index.event_id (an INTEGER
-- PRIMARY KEY = SQLite implicit rowid alias). Append-only PRINCIPLES #3 means we
-- never UPDATE / DELETE session_search_index in normal operation, so only the AI
-- trigger fires in steady state; ad / au triggers exist for the recovery/compaction
-- edge case.
CREATE TRIGGER session_search_index_ai AFTER INSERT ON session_search_index BEGIN
  INSERT INTO session_search_fts(rowid, searchable_text)
  VALUES (new.event_id, new.searchable_text);
END;
CREATE TRIGGER session_search_index_ad AFTER DELETE ON session_search_index BEGIN
  INSERT INTO session_search_fts(session_search_fts, rowid, searchable_text)
  VALUES ('delete', old.event_id, old.searchable_text);
END;
CREATE TRIGGER session_search_index_au AFTER UPDATE ON session_search_index BEGIN
  INSERT INTO session_search_fts(session_search_fts, rowid, searchable_text)
  VALUES ('delete', old.event_id, old.searchable_text);
  INSERT INTO session_search_fts(rowid, searchable_text)
  VALUES (new.event_id, new.searchable_text);
END;
```

Per-EventType `searchable_text` denormalization shape (F-3.16) — the extractor function
collapses each event's `data` JSON into a single tokenizable text blob in this shape so
FTS5 can index uniformly across all 8 variants:

| EventType   | searchable_text contents                                              | Phase 3 writer? |
|-------------|------------------------------------------------------------------------|-----------------|
| Message     | role + " " + text body                                                 | yes (existing)  |
| Action      | tool_name + " " + flattened tool_input parameter values                | yes (existing)  |
| Observation | tool_name + " " + truncated tool_result (cap at 4 KB)                  | yes (existing)  |
| Plan        | goal + " " + phases[].title joined by spaces                           | yes (existing)  |
| Skill       | kind + " " + playbook_id + " " + matcher_mode (when present)           | yes (new)       |
| Misc        | kind + " " + serialized reason / category fields per kind — Phase 3 new Misc kinds indexed: `playbook_extraction_*` (F-3.13/F-3.14/F-3.18/F-3.7 + NFR-3.1/3.4/3.5), `playbook_injection_truncated` (NFR-3.3), `session_search_summary_degraded` (§8) | yes (existing)  |
| Knowledge   | (reserved; Phase 4+ writer fills with cross-source-verified fact text) | no (Phase 4+)   |
| Datasource  | (reserved; Phase 4+ writer fills with URL + extracted-text excerpt)    | no (Phase 4+)   |

Pinned NFR budgets:
- NFR-3.3 injection cap: `12_288 bytes` aggregate across injected top-3 payload.
- NFR-3.4 extraction input cap: `8_192 tokens` (prompt + context).
- NFR-3.5 extraction output cap: `24_576 bytes`.

Quality-floor field (F-3.18): `playbooks.content` (single-field choice per F-3.18's
"e.g. `procedure_steps` or `content`" allowance). Extraction-pipeline order is fixed:

1. LLM returns structured output `{title, trigger_keywords[], overview, steps[]}`
   (F-3.14 layer-1 LLM abstraction applies here).
2. Deterministic PII redaction (F-3.14 layer 2) rewrites `overview` + `steps[]` in place.
3. Deterministic adversarial scan (F-3.13) — reject + emit on hit; skip write.
4. Quality-floor structural check on the redacted pre-render `steps[]` structure
   (NOT a post-render regex parse of `content`):
   - `steps.len() >= 3` (at least 3 non-trivial ordered steps; "non-trivial" = each step
     is at least one non-whitespace word after redaction).
   - `steps.join("\n").len() >= 200` UTF-8 characters after deterministic redaction.
   - Floor fail → emit `playbook_extraction_rejected{layer:"quality_floor",reason}`;
     skip write.
5. Render: `content = overview + "\n\n## Procedure\n" + numbered(steps)` so FTS5
   indexing covers BOTH narrative and step text.
6. NFR-3.5 cap (24_576 bytes) applies to the rendered `content`. If the cap fires AND
   post-cap content drops below the quality floor (rare; only triggers when overview +
   steps render exceeds 24 KB), emit BOTH `playbook_extraction_output_capped` AND
   `playbook_extraction_rejected{layer:"quality_floor"}` so the audit trail captures
   the actual failure chain (per iteration-5's `ExtractionOutcome::Skipped(OutputCapped)`
   variant in §4).

## 4. API surface

Internal/core APIs:
- `extract_playbook_sync(task_id, session_id) -> Result<ExtractionOutcome>` —
  `ExtractionOutcome ∈ {Written(playbook_id), Skipped(reason)}`. `Written` means the
  draft passed all gates AND the row was committed; `Skipped` means an extraction-pipeline
  `Misc` event was emitted and no row was written (used for timeout / error / rejected /
  output_capped+quality-floor-fail / etc.). The handler must never block task completion.
- `match_playbooks(project_id, brief, mode: MatcherMode) -> Vec<MatchedPlaybook>` —
  returns at most the top-3 set after threshold + tie-break (NFR-3.2). Always returns
  rows in deterministic order so the caller can rely on `[0]` being the top match.
- `inject_playbooks(system_prefix: &str, matches: &[MatchedPlaybook]) -> InjectedBlock` —
  pure function over the Initializer's existing system-prompt string; returns the
  augmented prefix plus the `injected_ids` / `total_bytes` / `truncated` metadata that
  feeds the `Skill{kind:"injection"}` event.
- `record_playbook_outcome(task_id, verifier_verdict) -> ()` — reads the task's
  injection set from the events table (single `Skill{kind:"injection",
  injected_ids:[...]}` row for this task; no separate `task_playbook_injections`
  table — events stay the single source of truth per ARCH §2.1 append-only). Filters
  internally to `pass`/`fail` verdicts (no-op for `error`/`skipped`); updates
  `success_count`/`failure_count` on each injected playbook AND emits one
  `Skill{kind:"outcome"}` per injected playbook in the same transaction.
- `index_event_for_search(event)` — invoked inline from the event-append path inside
  the SAME SQLite transaction as the event insert (`BEGIN` … `INSERT INTO events` …
  `INSERT INTO session_search_index` … `COMMIT`). This keeps replay/recovery consistent
  per the append-only PRINCIPLES #3 contract: no partial state where an event exists
  without its search row, and no orphan search row pointing at a missing event.
- `search_session_events(query, filters, limit) -> Vec<EventHit>` — FTS5 over
  `session_search_fts` with optional `event_type` / `source` / time-range filters.

Type sketches (PM persona derives field-level stories from these):

```rust
enum MatcherMode { Gate, Production }

struct MatchedPlaybook {
  playbook_id: String,
  project_id: String,           // F-3.12 enforcement (= task.project_id)
  matcher_mode: MatcherMode,    // copied into Skill{kind:"match"}
  match_score: f64,             // post-recency, post-weight; threshold `>= 1.0`
  // ranking-pipeline tie-break inputs (NFR-3.2):
  success_count: i64,
  failure_count: i64,
}

struct InjectedBlock {
  rendered_prefix: String,      // the augmented system prompt
  injected_ids: Vec<String>,    // 0..=3 playbook ids actually written into prefix
  total_bytes: usize,           // after NFR-3.3 cap application
  truncated: bool,              // true iff NFR-3.3 cap fired
}

enum ExtractionOutcome {
  Written(String /* playbook_id */),
  Skipped(SkipReason),
}
enum SkipReason {
  Timeout, Error, RejectedLlm, RejectedDeterministic, QualityFloor,
  OutputCapped /* and quality floor failed post-cap */,
}

struct EventHit {
  event_id: i64,
  session_id: String,
  timestamp: i64,
  event_type: String,           // 'Message' | 'Action' | ... | 'Misc'
  source: String,
  snippet: String,              // FTS5 snippet() helper output around the query terms
}
```

Tool un-stubs — tool input / output shapes (LLM-callable surface):
- `sop_read({id: string} OR {title: string})` →
  `{id, title, content, version, enforced} OR null`. Exact-match only in Phase 3
  (FTS5 over SOPs is Phase 4+). Both keys provided: `id` wins (more specific);
  neither provided: validation error before SQL.
- `playbook_search({query: string, limit?: number = 3})` →
  `[{playbook_id, title, content_excerpt (<= 512 bytes), match_score}, ...]`.
  Project-scoped to the calling session's task per F-3.12; archived rows excluded per
  §2.2 matcher contract. Limit clamped to `<= 3` (consistent with F-3.11 injection cap).
- `glossary_lookup({term: string})` →
  `{term, definition, category} OR null`. Exact term lookup; FTS5-backed fallback over
  `term` + `definition` if exact misses (so "headcount" finds "head count" entry).

CLI required surfaces:
- `seasoned-hand sop create|edit|list|show|delete` —
  hard-delete (DELETE row); SOPs are human-authored, audit lives in event-stream
  Misc events emitted by the CLI.
- `seasoned-hand playbook list|show|delete` —
  soft-delete (`UPDATE playbooks SET status='archived'`); preserves the row so the
  audit / counter history survives.
- `seasoned-hand session search <query>` (raw hits + summarized view).

HTTP API surface: Phase 3 ships NO new HTTP routes. NFR-3.6's "CLI/API queryability"
requirement is satisfied by the CLI surface above. HTTP routes for browser-side SOP /
playbook / search UX are Phase 4+ scope (paired with Curator + frontend work). Phase 2
DEBT #52 (lib.rs split) is therefore NOT a Phase 3 blocker for this functionality.

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
  Scope: outcome events fire for the playbooks that were INJECTED into this task
  (i.e. the top-3 set, minus any truncated by NFR-3.3). A playbook that was matched
  but truncated out of the injection window emits no outcome event — the task didn't
  consume it. This is why §4 `record_playbook_outcome` reads the injection set, not
  the match set.

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
- Event append path now also writes the denormalized session-search row for every
  EventType (per F-3.16's 8-variant coverage); Phase 3's new event volume comes from
  `Skill{kind:"match"|"injection"|"outcome"}` and the `playbook_extraction_*` `Misc`
  family — both must reach `session_search_index` so operators can grep them via the
  CLI surface in §4.
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
- FTS5 maintenance write amplification: every UPDATE on `playbooks` fires the §3
  `playbooks_au` trigger (1 delete + 1 insert on `playbooks_fts`); every event append
  fires the `session_search_index_ai` trigger (1 insert on `session_search_fts`). Both
  are amortized by FTS5's compact index and well-suited to SQLite WAL; assume +0.5 ms
  per event append (negligible vs the 120 ms search query budget above). Phase 4 may
  batch-rebuild via `INSERT INTO ... VALUES('rebuild')` if amplification becomes a
  measurable issue with the post-Phase-3 event volume.

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
  - Return raw FTS hits; emit `Misc{kind:"session_search_summary_degraded",
    session_id, query_hash, reason}` so operators can grep degradation incidents.
    (NOT under the `playbook_extraction_*` prefix — it's a search-path event, not an
    extraction-pipeline event.)

## 9. Security considerations

Layered extraction safety per F-3.13/F-3.14 — Phase 3 ships BOTH layers; the LLM layer
is probabilistic and the deterministic layer is the audit floor.

Deterministic adversarial scan baseline (F-3.13 — MUST detect, at minimum):
1. shell substitution / metacharacters: backticks, `$(...)`, pipe-to-shell `| sh` / `| bash`
2. raw IPv4 / IPv6 literal hosts in URLs
3. prompt-injection trigger phrases (Architect-curated list — the F-3.13 baseline; phase
   stories may extend it):
   - "ignore previous instructions" / "disregard the above" / "forget everything"
   - "you are now" / "act as" / "from now on you are"
   - role-reversal patterns ("you are not the assistant, you are...", "I am the assistant")
   - system-prefix injection (`system:`, `<|im_start|>system`, `[INST]`)
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
3. Reconcile architecture spec in same slice. **V010 retains `content_path` from V009;
   ARCH §2.5 has no `content_path` column — Phase 3 inherits this divergence.** Per
   F-3.19, this requires successor ADR-012 (following ADR-011's drift-consolidation
   precedent) in the SAME PR slice, bumping ARCH §2.5 v1.1 → v1.2 to document:
   - `content_path` as a reserved-for-Phase-4-spill column (Phase 3 writes the
     empty-string sentinel `''`; no spill semantics active until Phase 4 needs them);
   - the V010 ALTER-TABLE shape (DEFAULT-backfilled NOT NULL columns added against
     V009's row) as the canonical schema;
   - the FTS5 maintenance triggers from §3 as part of the playbooks contract.
4. Backfill existing V009 rows:
   - `trigger_keywords='[]'`, `content=''`, `status='active'`, `version=1`.
   - Keep `content_path` as-is at the V009 value (NOT NULL inherited); Phase 3 writes
     `''` for new rows and never reads spill semantics. NFR-3.5's 24_576-byte hard cap
     on extraction output is small enough that inline `content` is always sufficient in
     Phase 3.
   - After backfilling `content`, rebuild `playbooks_fts` once via
     `INSERT INTO playbooks_fts(playbooks_fts) VALUES('rebuild')` so existing rows
     become searchable; subsequent INSERT / UPDATE / DELETE on `playbooks` is handled
     by the §3 triggers.
5. New tables (`sops`, `glossary`, `session_search_index`, `session_search_fts`) start
   empty at V010; no backfill required.
6. Update `scripts/spec-check.sh` (closes Phase 2 DEBT #62 carry-forward) to enforce:
   V010 migration file present, `sops` + `glossary` table references, CLI command
   groups (`sop`, `playbook`, `session search`) registered. ARCH version bump v1.1→v1.2
   gates a Phase 3-version check.

## 11. Testing strategy

- Unit
  - deterministic normalizer (NFD + lowercase + whitespace collapse + trim) — same
    instance used by gate matcher AND production matcher per F-3.4.
  - tie-break ordering function (NFR-3.2)
  - adversarial scan / redaction matchers (F-3.13 / F-3.14)
  - quality-floor validator on extractor's pre-render `steps[]` structure (F-3.18 —
    counts steps, joined-step-length after redaction)
  - FTS5 score threshold gate: row with computed `score < 1.0` is excluded from
    the result set even if it tokenized
  - recency_boost monotonicity: a playbook 10 days old outranks the same playbook
    re-created 40 days ago (catches days_since_now vs days_since_epoch regressions)
- Integration
  - sync extraction success/failure/timeout paths
  - production matcher FTS5 smoke (`phase3_production_matcher_smoke`)
  - tool un-stubs return real rows
  - session index ingestion + queryability across all 8 event types
  - matcher excludes `status='archived'` playbooks from both gate and production modes
    (regression for F-3.20 soft-delete contract)
  - matcher excludes cross-project playbooks (F-3.12) — seed playbooks under
    project A, assert that matching for a task under project B returns zero
  - zero-match injection skip emits no `Skill{kind:"injection"}` (F-3.11)
  - outcome event fires for INJECTED playbooks only, not for matched-but-truncated
    rows (F-3.8 scope)
  - FTS5 maintenance trigger correctness: UPDATE on playbooks.content makes the new
    text searchable AND removes the old text from results (catches a missing `ad`
    trigger half of `au`); DELETE removes the row from search hits; rebuild
    `INSERT INTO playbooks_fts(playbooks_fts) VALUES('rebuild')` produces the same
    result set as trigger-maintained state (anti-skew regression)
- Acceptance (Phase 3 gate — `cargo test phase3_warm_benchmark`)
  - `phase3_warm_benchmark` enforces `sessions.tool_calls <= 0.70 x cold_baseline`
    (F-3.3 / requirements §4). `cold_baseline` is a checked-in integer constant in
    the test source captured from the `phase2_overnight_default_path` fixture at the
    Phase 3 kickoff lineage (around `cc7d4f0`); the constant is the authoritative
    pre-learning baseline so the test is reproducible without re-running the cold
    fixture in CI. Re-baselining requires a deliberate PR that flips the constant +
    cites the new lineage SHA.
  - LLM extraction call in the warm-run setup is mocked via a fixture playbook
    pre-seeded into V010 (NOT a live Bifrost call) — the gate measures the
    INJECTION+MATCH effect on tool_calls, not extraction quality. Extraction
    quality is covered by integration tests (`sync extraction success/failure/timeout
    paths`) and quality-floor unit tests separately.
  - Test-side time is frozen via `mock_clock::freeze(t0)` so `days_since_now` in the
    §11 recency formula is deterministic. The pre-seeded fixture playbook's
    `created_at = t0 - 5 days` keeps recency_boost = `max(0, 0.5 - 5/60) ≈ 0.417`,
    well above the `score >= 1.0` threshold floor regardless of weight tuning.
- Regression guards (separate from acceptance gate per F-3.6)
  - `sessions_tool_calls_matches_action_count` — parity test validating
    `sessions.tool_calls` wiring integrity against the cold baseline. NOT part of
    warm-gate success criteria (F-3.6); a counter-drift failure here fails CI but
    distinguishes "metric trust broken" from "learning regression".
  - `phase3_production_matcher_smoke` — seeds known playbook rows and asserts the
    FTS5 production matcher returns the expected top-3 on representative queries
    (requirements §4). Without this, `phase3_warm_benchmark` only exercises the gate
    matcher and the production matcher could ship broken.

Deterministic ranking / tie-break (NFR-3.2 — same ordering used by gate and production
matchers so behavior is consistent across modes):
1. Primary: `match_score DESC` (NFR-3.2 primary key)
2. Secondary: `(success_count - failure_count) DESC` (NFR-3.2 baseline secondary)
3. Tertiary: `success_count DESC` — breaks ties where `(success - failure)` matches
   between e.g. `(10-5)=(20-15)`; prefers the playbook with more proven uses regardless
   of failure count.
4. Quaternary: `playbook_id ASC` (stable deterministic final key — NFR-3.2 mandates a
   final stable key; UUID string compare is total-ordered).

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
- Inline only (chosen for Phase 3): simplest read/search; NFR-3.5's 24 KB output cap
  bounds row size at safe levels. `content_path` from V009 is retained as a reserved
  Phase-4-spill column (written as `''` in Phase 3); ADR-012 in the same V010 PR
  slice (§10.3) documents this.
- Path-only: compact rows but FTS5 needs a denormalized search column anyway.
- Active inline+spill hybrid: defers to Phase 4 when curation produces larger composed
  playbooks beyond the NFR-3.5 cap; activates the reserved `content_path`.
3. Injection site
- Initializer one-shot (chosen): zero extra per-iteration cost, matches F-3.11/NFR-3.2.
- Sticky every iteration: better persistence but ongoing token tax.
4. F-3.18 quality-floor field
- Single `content` field (chosen): F-3.18 explicitly allows this ("e.g. `procedure_steps`
  or `content`"); structural check runs on the extractor's pre-render `steps[]` JSON,
  not a post-render regex parse — cleaner contract.
- Separate `procedure_body` column: rejected. Would either duplicate procedure text
  (also stored in `content`) or split it from FTS5 indexing (F-3.5 indexes only
  `content`), making the production matcher miss step text.

## 13. Requirements coverage map

| Requirement              | Architecture section(s)                          |
|--------------------------|--------------------------------------------------|
| F-3.1 / ADR-007 1+2 only | §2.1 LearningExtractor, §10.1                    |
| F-3.2 benchmark fixture  | §11 Acceptance                                   |
| F-3.3 0.70× gate         | §11 Acceptance                                   |
| F-3.4 normalization      | §2.2 PlaybookMatcher                             |
| F-3.5 two matchers       | §2.2 PlaybookMatcher, §11 FTS5 scoring           |
| F-3.6 canonical KPI      | §11 Regression guards                            |
| F-3.7 sync execution     | §2.1, §4 extraction_error event, §6, §8          |
| F-3.8 Skill telemetry    | §4 event payloads (incl. injected-scope), §6 outcome filter |
| F-3.9 no curator         | §9 "no auto-archive", §12 deferred list          |
| F-3.10 SOP surface       | §2.5 CLI, §3 sops table, §4 sop_read un-stub     |
| F-3.11 top-3 injection   | §2.3 PlaybookInjector, §11 ranking pipeline      |
| F-3.12 project scope     | §9 security controls                             |
| F-3.13 adversarial scan  | §9 deterministic baseline                        |
| F-3.14 PII redaction     | §9 PII baseline                                  |
| F-3.15 immediate activation | §9 (no quarantine)                            |
| F-3.16 search index 8x   | §3 session_search_index + per-type shape table + CHECK |
| F-3.17 summarization     | §2.4 SessionSearchIndex, §7 summarizer slot      |
| F-3.18 quality floor     | §3 quality-floor field, §11 unit tests           |
| F-3.19 atomic slice      | §10 Migration plan, §3 FTS5 triggers (covered by ADR-012) |
| F-3.20 playbook CLI      | §2.5, §4 CLI surfaces                            |
| F-3.21 glossary surface  | §3 glossary table, §4 glossary_lookup un-stub    |
| NFR-3.1 60s timeout      | §7, §8                                           |
| NFR-3.2 deterministic    | §11 tie-break ordering                           |
| NFR-3.3 injection cap    | §3 pinned budget, §4 injection_truncated event   |
| NFR-3.4 input cap        | §3 pinned budget, §4 input_truncated event       |
| NFR-3.5 output cap       | §3 pinned budget, §4 output_capped event         |
| NFR-3.6 search operability | §2.4, §4 CLI/API surface                       |
