# Phase 4 — Architecture (Curator + Self-Improvement)

Date: 2026-05-18  
Owner: BMAD Architect pass  
Status: v1.0 draft (replaces placeholder)

This document translates `/specs/phase-4/requirements.md` into concrete architecture for
implementation. It follows the same depth model as Phase 3 architecture and resolves all 20
questions from `/specs/phase-4/OPEN_QUESTIONS.md`.

## 1. Summary diagram

```text
                                  +-------------------------------+
                                  |      Operator Review Queue    |
                                  | approve/reject/suppress/mute  |
                                  +---------------+---------------+
                                                  |
                                                  v
+--------------------+      +---------------------+---------------------+
| Verifier PASS path |----->|   CuratorWorker (async, non-blocking)    |
| (Phase 3 complete) |      |   interval + backlog triggers             |
+--------------------+      +--+----------+------------+-----------+----+
                               |          |            |           |
                               |          |            |           |
                               v          v            v           v
                      +--------+--+ +-----+------+ +---+-----+ +---+----------------+
                      |Candidate  | |Embedding    | |Consoli- | |ConflictDetector     |
                      |Builder    | |Reranker     | |dation   | |(SOP contradiction)  |
                      +-----+-----+ +------+------+ |Engine    +---+----------------+
                            |              |        +---+-----------+
                            |              |            |
                            v              v            v
                     +------+--------------+------------+-------------------+
                     | curator_decisions ledger + playbook_revisions graph |
                     | knowledge_items + datasource_items + conflict_items  |
                     +------+----------------------+-------------------------+
                            |                      |
                            |                      |
                            v                      v
                     +------+------------------+  +-------------------------+
                     | playbooks (active set)  |  | weekly_retrospectives    |
                     | + playbooks_fts         |  | citation-anchored output |
                     +------+------------------+  +-------------------------+
                            |
                            v
                 +----------+-------------------+
                 | Initializer injector/matcher |
                 | (Phase 3 surfaces reused)    |
                 +------------------------------+
```

Architecture intent:
- Curator is asynchronous and never blocks verifier completion (F-4.1, NFR-4.2).
- Decisions are durable, explainable, and reversible through a ledger + review queue.
- Phase 3 learning artifacts remain the input substrate; Phase 4 adds curation intelligence,
  consolidation, retrospective synthesis, and operator governance.

## 2. New components introduced

### 2.1 CuratorWorker

Purpose:
- Background orchestrator for curation cycles.
- Enforces trigger policy, run budget, idempotency, and failure isolation boundaries.

Technology:
- Rust Tokio task spawned from server boot (same style as verifier/checkpoint workers).
- SQLite transactional writes + Redis stream notification for review queue updates.

Integration points:
- Reads recent Phase 3 extraction outcomes + Skill events (`match`, `injection`, `outcome`).
- Writes `curator_decisions`, `playbook_revisions`, conflict tables, retrospective tables.
- Emits `Misc{kind:"curator_*"}` telemetry and optional `Skill{kind:"curation_decision"}`.

### 2.2 CandidateBuilder

Purpose:
- Build deterministic candidate sets for duplicate detection, archive analysis, and conflict checks.

Technology:
- SQL candidate prefilter + FTS5 rank windows + deterministic tie-break ordering.

Integration points:
- Reuses Phase 3 matcher/provenance concepts and project scoping.
- Produces bounded candidate batches for EmbeddingReranker.

### 2.3 EmbeddingReranker

Purpose:
- Semantic rerank for consolidation/relatedness decisions over FTS shortlist (closes DEBT #72).

Technology:
- Uses existing `embedding` slot from model routing (now wired for production usage).
- Batch embedding requests with cache and cost accounting.

Integration points:
- Consumes candidates from CandidateBuilder.
- Supplies similarity matrix to ConsolidationEngine and ConflictDetector.

### 2.4 ConsolidationEngine

Purpose:
- Decide merge vs keep vs quarantine vs auto-archive recommendation.
- Apply revision updates with provenance links and rollback capability.

Technology:
- Deterministic policy engine + transaction-bound write set.

Integration points:
- Updates `playbooks` active pointers and `playbook_revisions` lineage.
- Creates review items for low-confidence actions.

### 2.5 ConflictDetector

Purpose:
- Detect SOP contradictions and conflicting procedural guidance (F-4.10/F-4.11).

Technology:
- Structural-step overlap prefilter + semantic contradiction adjudication.

Integration points:
- Writes `sop_conflicts` and raises review items for severe/ambiguous conflicts.

### 2.6 RetrospectiveGenerator

Purpose:
- Produce weekly evidence-cited retrospectives with refusal on weak evidence (F-4.17/F-4.18).

Technology:
- Session-search summarization path + structured citation extraction validator.

Integration points:
- Reads `curator_decisions`, `sop_conflicts`, outcome metrics, and session_search references.
- Writes `weekly_retrospectives` + `retrospective_citations`.

### 2.7 WorkPatternExtractor

Purpose:
- Mine recurring operational patterns from event streams and outcomes (F-4.19/F-4.20).

Technology:
- Hybrid replay window + pre-aggregated stats.

Integration points:
- Feeds recommendations into Curator decision loop and review queue.

### 2.8 OperatorReviewQueue

Purpose:
- Gate low-confidence/high-impact actions with human accept/reject/suppress controls.

Technology:
- New tables + CLI surfaces; can later back into frontend UI.

Integration points:
- Consumes generated review items from ConsolidationEngine/ConflictDetector.
- Writes disposition actions and feeds policy learning telemetry.

## 3. Data model changes (V011)

Phase 4 requires V011 migration to close F-4.14 and provide durable curation surfaces.

### 3.1 V011 goals

- Add revision identity model for F-4.7 and downstream F-4.3/F-4.6/F-4.20 coherence.
- Add Curator decision ledger and operator review queue.
- Add Knowledge/Datasource write tables and conflict tables.
- Add retrospective storage and citation references.
- Denormalize `source_project_id` into `playbooks` (DEBT #77).

### 3.2 Schema SQL sketch (authoritative shape for implementation stories)

> **Forward-compat note (REVIEW iter-1 F2)**: every new Phase 4 table below MUST include a
> nullable `tenant_id TEXT` column at the same column position pattern that V009 used for
> `skills` and `playbooks`. Phase 4 writers write `NULL` (single-operator scope). Phase 5
> multi-user will flip these to `NOT NULL` with backfill per the ADR-013 pattern. Without
> this forward-compat column now, Phase 5 will require destructive schema migration of
> every Phase 4 table. The sketch below shows the column on each CREATE TABLE; existing
> `playbooks` already has `tenant_id` from V009.

```sql
-- 1) playbooks denormalization + revision pointer
ALTER TABLE playbooks ADD COLUMN source_project_id TEXT;
ALTER TABLE playbooks ADD COLUMN active_revision_id TEXT;
ALTER TABLE playbooks ADD COLUMN archived_reason TEXT;
ALTER TABLE playbooks ADD COLUMN archived_at INTEGER;

CREATE INDEX idx_playbooks_project_status ON playbooks(source_project_id, status);

-- 2) revision graph (chosen F-4.7 model)
CREATE TABLE playbook_revisions (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
  playbook_id TEXT NOT NULL,
  revision_no INTEGER NOT NULL,
  parent_revision_id TEXT,
  title TEXT NOT NULL,
  trigger_keywords TEXT NOT NULL DEFAULT '[]',
  content TEXT NOT NULL,
  source_task_id TEXT,
  source_project_id TEXT NOT NULL,
  author_type TEXT NOT NULL CHECK(author_type IN ('human','curator','extractor')),
  change_kind TEXT NOT NULL CHECK(change_kind IN ('extract','merge','improve','archive','restore')),
  confidence REAL,
  created_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000000 AS INTEGER)),
  superseded_at INTEGER,
  UNIQUE(playbook_id, revision_no),
  FOREIGN KEY(playbook_id) REFERENCES playbooks(id)
);
CREATE INDEX idx_playbook_revisions_playbook ON playbook_revisions(playbook_id, revision_no DESC);
CREATE INDEX idx_playbook_revisions_project ON playbook_revisions(source_project_id, created_at DESC);

-- 3) revision-scoped success/failure metrics
CREATE TABLE playbook_revision_outcomes (
  revision_id TEXT PRIMARY KEY,
  tenant_id TEXT,
  success_count INTEGER NOT NULL DEFAULT 0,
  failure_count INTEGER NOT NULL DEFAULT 0,
  decayed_success REAL NOT NULL DEFAULT 0,
  decayed_failure REAL NOT NULL DEFAULT 0,
  last_outcome_at INTEGER,
  FOREIGN KEY(revision_id) REFERENCES playbook_revisions(id)
);

-- 4) curator decision ledger (append-only)
CREATE TABLE curator_decisions (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
  project_id TEXT NOT NULL,
  cycle_id TEXT NOT NULL,
  decision_type TEXT NOT NULL CHECK(decision_type IN (
    'merge','keep','archive','restore','conflict_raise','retrospective','recommendation','knowledge_write','datasource_write'
  )),
  subject_kind TEXT NOT NULL CHECK(subject_kind IN ('playbook','revision','conflict','retrospective','pattern','knowledge','datasource')),
  subject_id TEXT NOT NULL,
  confidence REAL,
  rationale_json TEXT NOT NULL,
  evidence_json TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('applied','queued_review','rejected','suppressed','error')),
  failure_category TEXT,
  created_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000000 AS INTEGER))
);
CREATE INDEX idx_curator_decisions_project_time ON curator_decisions(project_id, created_at DESC);
CREATE INDEX idx_curator_decisions_cycle ON curator_decisions(cycle_id);

-- 5) review queue
CREATE TABLE curator_review_queue (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
  decision_id TEXT NOT NULL,
  project_id TEXT NOT NULL,
  queue_reason TEXT NOT NULL,
  severity TEXT NOT NULL CHECK(severity IN ('high','medium','low')),
  state TEXT NOT NULL CHECK(state IN ('pending','approved','rejected','suppressed')),
  reviewer TEXT,
  reviewer_note TEXT,
  resolved_at INTEGER,
  created_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000000 AS INTEGER)),
  FOREIGN KEY(decision_id) REFERENCES curator_decisions(id)
);
CREATE INDEX idx_curator_review_pending ON curator_review_queue(project_id, state, created_at DESC);

-- 6) conflict artifacts
CREATE TABLE sop_conflicts (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
  project_id TEXT NOT NULL,
  left_revision_id TEXT NOT NULL,
  right_revision_id TEXT NOT NULL,
  structural_score REAL NOT NULL,
  semantic_score REAL NOT NULL,
  severity TEXT NOT NULL CHECK(severity IN ('low','medium','high')),
  status TEXT NOT NULL CHECK(status IN ('open','muted','resolved')),
  evidence_json TEXT NOT NULL,
  created_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000000 AS INTEGER))
);
CREATE INDEX idx_sop_conflicts_project_status ON sop_conflicts(project_id, status, created_at DESC);

-- 7) Knowledge + Datasource writers
CREATE TABLE knowledge_items (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
  project_id TEXT NOT NULL,
  revision_id TEXT,
  source_task_id TEXT,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  confidence REAL,
  evidence_json TEXT NOT NULL,
  created_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000000 AS INTEGER))
);
CREATE INDEX idx_knowledge_items_project_key ON knowledge_items(project_id, key);

CREATE TABLE datasource_items (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
  project_id TEXT NOT NULL,
  revision_id TEXT,
  source_task_id TEXT,
  source_type TEXT NOT NULL,
  source_ref TEXT NOT NULL,
  trust_level TEXT NOT NULL CHECK(trust_level IN ('l0','l1','l2')),
  confidence REAL,
  evidence_json TEXT NOT NULL,
  created_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000000 AS INTEGER))
);
CREATE INDEX idx_datasource_items_project_type ON datasource_items(project_id, source_type, created_at DESC);

-- 8) weekly retrospectives + citations
CREATE TABLE weekly_retrospectives (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
  project_id TEXT NOT NULL,
  -- week_start/week_end stored as microsecond INTEGER timestamps for sort + cross-table
  -- compatibility with events table (ARCH §2.1). Operators see human-readable form via
  -- CLI/UI conversion at display time.
  week_start INTEGER NOT NULL,
  week_end INTEGER NOT NULL,
  content TEXT NOT NULL,
  citation_coverage REAL NOT NULL,
  generation_status TEXT NOT NULL CHECK(generation_status IN ('success','refused','error')),
  created_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000000 AS INTEGER)),
  UNIQUE(project_id, week_start, week_end)
);
CREATE INDEX idx_weekly_retrospectives_project_week ON weekly_retrospectives(project_id, week_end DESC);

CREATE TABLE retrospective_citations (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
  retrospective_id TEXT NOT NULL,
  claim_index INTEGER NOT NULL,
  citation_kind TEXT NOT NULL CHECK(citation_kind IN ('event','decision','conflict','task')),
  citation_ref TEXT NOT NULL,
  snippet TEXT,
  FOREIGN KEY(retrospective_id) REFERENCES weekly_retrospectives(id)
);
CREATE INDEX idx_retrospective_citations_retrospective ON retrospective_citations(retrospective_id, claim_index);
```

### 3.3 FTS5/search impacts

- `playbooks_fts` remains keyed to active playbook content; `source_project_id` is not added to
  FTS columns (project scoping is deterministic WHERE-filter, not ranking signal).
- New searchable text surfaces for review UX use dedicated FTS virtual table:

```sql
CREATE TABLE curator_search_index (
  row_id INTEGER PRIMARY KEY,
  tenant_id TEXT,
  project_id TEXT NOT NULL,
  source_type TEXT NOT NULL,
  source_id TEXT NOT NULL,
  searchable_text TEXT NOT NULL,
  created_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000000 AS INTEGER))
);

CREATE VIRTUAL TABLE curator_search_fts USING fts5(
  searchable_text,
  content='curator_search_index',
  content_rowid='row_id',
  tokenize='porter unicode61'
);

CREATE TRIGGER curator_search_index_ai AFTER INSERT ON curator_search_index BEGIN
  INSERT INTO curator_search_fts(rowid, searchable_text)
  VALUES (new.row_id, new.searchable_text);
END;

CREATE TRIGGER curator_search_index_ad AFTER DELETE ON curator_search_index BEGIN
  INSERT INTO curator_search_fts(curator_search_fts, rowid, searchable_text)
  VALUES ('delete', old.row_id, old.searchable_text);
END;

CREATE TRIGGER curator_search_index_au AFTER UPDATE ON curator_search_index BEGIN
  INSERT INTO curator_search_fts(curator_search_fts, rowid, searchable_text)
  VALUES ('delete', old.row_id, old.searchable_text);
  INSERT INTO curator_search_fts(rowid, searchable_text)
  VALUES (new.row_id, new.searchable_text);
END;
```

### 3.4 Backfill plan (V011)

1. Fill `playbooks.source_project_id` by joining `playbooks.source_task_id -> tasks.project_id`.
2. Seed initial revision rows from existing `playbooks` rows (`revision_no=1`, `author_type='extractor'`).
3. Set `playbooks.active_revision_id` to seeded revision ids.
4. Seed `playbook_revision_outcomes` from existing `success_count/failure_count`.
5. Rebuild `playbooks_fts` once post-backfill:
   `INSERT INTO playbooks_fts(playbooks_fts) VALUES('rebuild');`
6. Commit in one migration transaction; failures roll back fully.

## 4. API surface and type sketches

### 4.1 Core Curator interfaces

```rust
pub struct CuratorConfig {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub backlog_threshold: u32,
    pub max_candidates_per_cycle: u32,
    pub embedding_budget_monthly_tokens: u64,
    pub embedding_budget_percent_cap: f32,
    pub auto_archive_enabled: bool,
    pub auto_merge_enabled: bool,
    pub retrospectives_enabled: bool,
}

pub trait CuratorWorker {
    async fn run(&self, cancel: CancellationToken) -> anyhow::Result<()>;
    async fn run_once(&self, trigger: CuratorTrigger) -> anyhow::Result<CuratorCycleResult>;
}

pub enum CuratorTrigger {
    IntervalTick,
    BacklogThreshold,
    Manual,
}

pub struct CuratorCycleResult {
    pub cycle_id: String,
    pub project_id: String,
    pub decisions_total: u32,
    pub queued_for_review: u32,
    pub failures: u32,
    pub elapsed_ms: u64,
}
```

### 4.2 Candidate + rerank contracts

```rust
pub struct DuplicateCandidate {
    pub left_revision_id: String,
    pub right_revision_id: String,
    pub fts_score: f32,
    pub lexical_overlap: f32,
    pub recency_delta_days: i32,
}

pub struct RerankedCandidate {
    pub left_revision_id: String,
    pub right_revision_id: String,
    pub blended_score: f32,
    pub embedding_cosine: f32,
    pub fts_norm: f32,
}

pub trait CandidateBuilder {
    async fn build_duplicate_candidates(
        &self,
        project_id: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<DuplicateCandidate>>;
}

pub trait EmbeddingReranker {
    async fn rerank(
        &self,
        project_id: &str,
        candidates: Vec<DuplicateCandidate>,
    ) -> anyhow::Result<Vec<RerankedCandidate>>;
}
```

Embedding wiring contract (load-bearing F-4.5 pin):
- Endpoint: Bifrost embeddings API (`POST /v1/embeddings`) via existing Rust LLM client surface.
- Default model profile: OpenAI `text-embedding-3-small` (configurable via `SH_EMBEDDING_MODEL`).
- Candidate input: normalized `title + trigger_keywords + content` text per revision.
- Blend formula: `blended_score = 0.45 * fts_norm + 0.40 * embedding_cosine + 0.15 * structural_overlap`.
- Deterministic fallback: if embedding call fails/disabled/budget-circuit-open, use
  `0.75 * fts_norm + 0.25 * structural_overlap` and mark decision rationale with
  `embedding_used=false`.
- Cache location: in-process bounded LRU keyed by `{revision_id, content_hash, model}` plus
  optional SQLite `curator_embedding_cache` table for warm-start reuse across restarts.

### 4.3 Consolidation + review contracts

```rust
pub enum ConsolidationDecisionKind {
    Merge,
    Keep,
    ArchiveRecommend,
    ArchiveApply,
    Quarantine,
}

pub struct ConsolidationDecision {
    pub decision_id: String,
    pub kind: ConsolidationDecisionKind,
    pub subject_revision_ids: Vec<String>,
    pub target_revision_id: Option<String>,
    pub confidence: f32,
    pub rationale_json: serde_json::Value,
    pub requires_review: bool,
}

pub trait ConsolidationEngine {
    async fn decide(
        &self,
        project_id: &str,
        reranked: Vec<RerankedCandidate>,
    ) -> anyhow::Result<Vec<ConsolidationDecision>>;

    async fn apply(
        &self,
        project_id: &str,
        decisions: &[ConsolidationDecision],
    ) -> anyhow::Result<()>;
}
```

### 4.4 Conflict + retrospective contracts

```rust
pub struct ConflictCandidate {
    pub left_revision_id: String,
    pub right_revision_id: String,
    pub structural_score: f32,
}

pub struct ConflictFinding {
    pub conflict_id: String,
    pub severity: ConflictSeverity,
    pub structural_score: f32,
    pub semantic_score: f32,
    pub evidence_json: serde_json::Value,
}

pub enum ConflictSeverity { Low, Medium, High }

pub trait ConflictDetector {
    async fn detect(&self, project_id: &str) -> anyhow::Result<Vec<ConflictFinding>>;
}

pub struct WeeklyRetrospective {
    pub retrospective_id: String,
    pub project_id: String,
    pub week_start: i64,
    pub week_end: i64,
    pub content: String,
    pub citation_coverage: f32,
}

pub trait RetrospectiveGenerator {
    async fn generate_weekly(&self, project_id: &str) -> anyhow::Result<WeeklyRetrospective>;
}
```

### 4.5 Event payload shape

Phase 4 keeps Phase 3 taxonomy discipline:
- `Skill` remains semantic learning events.
- `Misc` remains operational pipeline telemetry.

**Architecture taxonomy expansion (REVIEW iter-1 F3)**: Phase 3 F-3.8 pinned the
`Skill.kind` vocabulary to `{match, injection, outcome}`. Phase 4 ADDS the
`curation_decision` kind as a fourth canonical Skill sub-kind. This is an
architecture-level taxonomy extension; the V011 atomic-slice ADR-013 must document
the expanded F-3.8 vocab as `{match, injection, outcome, curation_decision}` and
update ARCH §2.1 event-kind enumeration alongside ARCH §2.5 v1.2 → v1.3 schema bump.
Downstream consumers (event-stream replay, session search index, dashboards) must
treat the new kind as first-class, not unknown.

New `Skill` kind:

```json
{
  "type": "Skill",
  "kind": "curation_decision",
  "project_id": "...",
  "decision_type": "merge|archive|keep|conflict_raise|recommendation",
  "subject_id": "...",
  "confidence": 0.87,
  "review_state": "applied|queued_review|rejected"
}
```

New `Misc` `curator_*` events:
- `curator_cycle_started{cycle_id,project_id,trigger}`
- `curator_cycle_completed{cycle_id,elapsed_ms,decisions_total,failures}`
- `curator_decision_quarantined{decision_id,failure_category,retry_count}`
- `curator_budget_circuit_open{budget_kind,month_tokens,pct_of_total}`
- `curator_retrospective_refused{project_id,reason}`

### 4.6 CLI surfaces

Phase 4 adds operator/PM-relevant CLI commands:

- `seasoned-hand curator status`
- `seasoned-hand curator run --project <id> [--dry-run]`
- `seasoned-hand curator review list [--project <id>] [--state pending]`
- `seasoned-hand curator review approve <queue_id> [--note ...]`
- `seasoned-hand curator review reject <queue_id> [--note ...]`
- `seasoned-hand curator review suppress <queue_id> [--ttl-days N]`

These map directly to OperatorReviewQueue and are story-slice friendly.

## 5. External dependencies

Planned dependency additions (subject to story-level exact versions):
- `cron` (or `tokio-cron-scheduler`) for robust weekly cadence handling.
- `ordered-float` for stable score ordering where needed.
- `lru` for embedding vector cache (in-process bounded cache).

No new service dependency is required; embedding calls route through existing Bifrost gateway.

Architecture policy:
- Any new crate introduced in implementation must update `ARCHITECTURE.md` §1 addendum in the same
  PR slice, following existing Phase 2/3 precedent.

## 6. Interactions with existing components

### 6.1 PlannerSlotExtractionHandler

- Curator does not call the extraction handler directly.
- It consumes outputs written by extraction handler + verifier-gated pass outcomes.
- This avoids duplicate extraction and preserves Phase 3 safety floor behavior.

### 6.2 VerifierGate integration

- Verifier PASS events are the canonical signal source for new curator backlog items.
- Curator failures never feed back into PASS/FAIL verdict state transitions.

### 6.3 Matcher + injector integration

- Consolidation updates affect active revision pointers used by matcher/injector.
- Matcher query semantics remain deterministic; archive status excludes candidates.
- Injector reads top-k from active revisions only and logs revision ids for outcome attribution.

### 6.4 Event store integration

- Existing append-only event stream principle remains intact.
- Curator writes only additive events; no mutation of historical stream entries.

### 6.5 `main.rs` wiring

- Add CuratorWorker bootstrap adjacent to verifier/checkpoint tasks.
- Add config parsing for `SH_CURATOR_*` flags with strict parser (closes DEBT #91 scope).

## 7. Performance budget

Per-cycle budget (NFR-4.1 alignment, <=50 candidates):

- CandidateBuilder: p95 <= 400ms
- EmbeddingReranker: p95 <= 1800ms (including cache lookups + API calls)
- ConsolidationEngine decide+apply: p95 <= 700ms
- ConflictDetector: p95 <= 700ms
- RetrospectiveGenerator (weekly path, amortized): p95 <= 1200ms
- Total Curator cycle: p95 <= 4000ms, p99 <= 8000ms

Task-path isolation:
- Additional synchronous work in verifier PASS path: <=50ms p95 (enqueue only).

Embedding cost ceiling mechanism (NFR-4.6):
- `monthly_embedding_tokens / monthly_total_tokens <= 0.08` soft cap.
- Hard stop at `0.12` opens `curator_budget_circuit_open`, forcing lexical-only fallback.
- For zero-baseline projects, absolute 50k token fallback budget applies.

## 8. Failure modes and handling

F-4.22 categories and handling:

1. Rust panic / propagated error
- Handler: catch at decision-unit boundary, mark `curator_decisions.status='error'`, emit
  `curator_decision_quarantined`, continue cycle.

2. LLM refusal
- Handler: classify refusal reason, record as non-fatal quarantine, optionally queue review when
  refusal blocks high-severity conflict path.

3. Malformed payload
- Handler: schema-validate every model response; invalid payload retries once with stricter format
  prompt, then quarantine.

4. Timeout (NFR-4.1 exceeded)
- Handler: per-call timeout budgets; one retry with reduced batch; then quarantine with
  `failure_category='timeout'`.

5. Out-of-memory
- Handler: split candidate batch size by half and retry once; on second OOM, abort current
  component run, mark cycle partial, continue next scheduled cycle.

6. SQLite lock contention (`BUSY` after retries)
- Handler: exponential backoff (50ms, 100ms, 200ms, 400ms) up to retry budget; then quarantine
  affected decision and continue.

7. Slot-router resolution failure
- Handler: open budget/fallback circuit to lexical-only mode, emit `curator_budget_circuit_open`
  with reason `slot_unavailable`, proceed without embeddings.

Global policy:
- No category above may fail the entire cycle unless config/schema bootstrap is invalid at startup.

## 9. Security considerations

- Project isolation: every curator read/write query must constrain `project_id` (F-4.24).
- Auto-archive safety: high-impact decisions require confidence threshold + optional review gating
  path, preserving NFR-4.7 bounds.
- Review queue access: CLI/API review actions require operator auth scope (same trust envelope as
  existing admin operations).
- PII handling: retrospective citations store bounded snippets and reuse Phase 3 redaction helpers.
- Prompt-injection resistance: Curator LLM prompts operate on filtered, redacted artifacts only;
  adversarial markers produce refusal/quarantine, not direct write.

### 9.1 Adversarial Curator scenario (REVIEW iter-1 F7)

An attacker may try to weaponize the Curator by manipulating the confidence-scoring
pipeline so a malicious merge/archive decision lands above the F-4.25 / §12.19 review
threshold (>0.75) and auto-applies without human gating.

Mitigation — confidence is composed from a deterministic floor + bounded LLM contribution,
not from LLM alone:

- **Deterministic signals (untrusted-input independent)**: FTS overlap score, structural
  step-diff similarity, lexical Jaccard, recency delta. Computed in-process from the
  candidate artifacts; not influenced by LLM output.
- **LLM signal (bounded contribution)**: embedding cosine + LLM-judged semantic agreement.
  Maximum LLM contribution to the blended `confidence` is capped at `0.45` (less than the
  review threshold delta of `0.75 - lowest_deterministic_floor`); means a malicious LLM
  score of `1.0` cannot, on its own, push a decision past the auto-apply threshold.
- **Floor enforcement**: if `max(deterministic_signals) < 0.30`, the candidate is
  short-circuited to "queue review" regardless of LLM agreement. An LLM convinced of a
  bad merge cannot bypass a low-floor candidate.
- **Adversarial input itself**: any artifact passing Phase 3 F-3.13 (deterministic
  adversarial scan) reaches the Curator already scrubbed. Curator's redaction reuse
  (per §9 bullet 4) further blunts injected payload influence on its own prompts.

Without these compositional bounds, a single compromised model run could effect
multiple auto-archive/merge decisions across the corpus in one cycle.

## 10. Migration and rollout plan (V011 atomic slice)

### 10.1 Atomic-slice rule

Phase 4 mirrors Phase 3 reconciliation discipline:
- V011 migration + production writer wiring + spec reconciliation must land in same PR slice.
- If `ARCHITECTURE.md` immutable architecture surfaces change materially (v1.2 -> v1.3), successor
  ADR-013 must land in that same PR slice.

### 10.2 V011 rollout steps

1. Add V011 SQL migration with schema from §3.
2. Backfill `source_project_id`, seed revision table, seed outcomes.
3. Rebuild `playbooks_fts` once.
4. Wire CuratorWorker boot path disabled by default (`SH_CURATOR_ENABLED=false`).
5. Enable in canary mode for selected project with review queue required on merge/archive actions.
6. Expand to default-on after NFR checks pass.

### 10.3 Reversibility

- Migration downgrade path uses snapshot/backup restore (no lossy downgrades promised).
- Runtime feature flags can disable auto-merge/auto-archive independently without rolling back schema.

## 11. Testing strategy

### 11.1 Unit tests

- CandidateBuilder deterministic ordering and tie-break behavior.
- EmbeddingReranker blend math and lexical fallback path.
- Consolidation policy thresholds and revision lineage writes.
- Conflict baseline agreement rule (structural + semantic).
- Retrospective citation coverage calculator (>=95% threshold).
- Config parser strictness for `SH_CURATOR_*` flags.

### 11.2 Integration tests

- V011 migration idempotency and backfill correctness.
- Review queue transitions (pending -> approved/rejected/suppressed).
- Failure containment per 7 categories (one test each).
- Knowledge/Datasource writer emit conditions + L2 enforcement.
- Project-isolation checks: cross-project rows cannot be touched.

### 11.3 Acceptance/benchmark tests

- `phase4_warm_full_loop_benchmark`: validate F-4.21 improvement proxy and budget caps.
- Weekly retrospective cadence/retry test meeting NFR-4.8.
- False-positive audit harness for auto-archive/merge (NFR-4.7 sample floor).

### 11.4 OPEN_QUESTIONS resolution tests

Each §12 decision creates at least one explicit regression test story in PM decomposition. No
question resolves only in prose.

## 12. OPEN_QUESTIONS resolutions (20/20)

### 12.1 Q1 Auto-archive threshold semantics

Chosen option: **B (project-level configurable thresholds)**.

Rationale:
- Better fit than global constants without heavy adaptive complexity.
- Works with review queue safety and phased rollout.

Deferred debt:
- Adaptive thresholding (option C) remains Phase 5 optimization candidate (DEBT #92).

### 12.2 Q2 Consolidation similarity metric strategy

Chosen option: **D (hybrid + structural-step alignment)**.

Rationale:
- Best protection against unrelated merges (NFR-4.7 critical).
- Embedding cost remains bounded because FTS shortlist still prefilters.

Deferred debt:
- None; this is the target architecture for Phase 4.

### 12.3 Q3 Consolidation write behavior

Chosen option: **B (new revision + supersede predecessors)**.

Rationale:
- Strong rollback/audit with manageable complexity.
- Avoids the autonomy slowdown of manual promotion-only path.

Deferred debt:
- Optional fork/promotion mode may be added later for high-regulated environments (DEBT #93).

### 12.4 Q4 Curator cycle trigger control

Chosen option: **C (dual trigger with guardrails)**.

Rationale:
- Meets freshness and backlog control requirements simultaneously.
- Deterministic arbitration pinned: backlog trigger wins only when pending items > threshold;
  otherwise interval trigger.

Deferred debt:
- None.

### 12.5 Q5 Curator failure isolation boundary

Chosen option: **C (per-decision-unit isolation)**.

Rationale:
- Directly supports F-4.22 and avoids cycle-level stalls.

Deferred debt:
- None.

### 12.6 Q6 Retrospective generation cadence

Chosen option: **B (weekly minimum + activity-triggered extras)**.

Rationale:
- Preserves roadmap weekly guarantee while allowing incident-sensitive extra runs.

Deferred debt:
- Extra-run throttling heuristics may require tuning.

### 12.7 Q7 Retrospective model slot choice

Chosen option: **B (dedicated summarizer profile within existing routing)**.

Rationale:
- Avoids planner-slot contention and provides predictable cost/quality knobs.
- Implemented as profile on existing `session_search` summarization path (no new 13th slot).

Deferred debt:
- Tiered model-by-size (option C) can be Phase 5 optimization (DEBT #94).

### 12.8 Q8 Work-pattern signal source

Chosen option: **C (hybrid replay window + aggregates)**.

Rationale:
- Best fidelity/cost balance.

Deferred debt:
- Long-horizon full replay analytics deferred to Phase 5 scaling work.

### 12.9 Q9 SOP conflict detection algorithm

Chosen option: **C (rule-first + semantic adjudication)**.

Rationale:
- Matches F-4.10 baseline while controlling cost and noise.

Deferred debt:
- None.

### 12.10 Q10 Skill self-improvement application mode

Chosen option: **B (mandatory revision chain)**.

Rationale:
- This is the load-bearing F-4.7 choice.
- Revision id becomes canonical key used by:
  - F-4.3 outcome metrics (`playbook_revision_outcomes.revision_id`)
  - F-4.6 consolidation decisions (`target_revision_id`)
  - F-4.20 recommendation provenance (`subject_revision_ids`)

Deferred debt:
- Optional promotion workflow for regulated domains.

### 12.11 Q11 Curator and `playbooks.status='archived'`

Chosen option: **B (extend archived semantics with reason/confidence)**.

Rationale:
- Keeps compatibility with Phase 3 while exposing curator intent.

Deferred debt:
- Separate archive table (option C) deferred unless audit throughput demands it.

### 12.12 Q12 Telemetry retention vs storage budget

Chosen option: **B (tiered retention raw -> summarized)**.

Rationale:
- Needed to satisfy 90-day retention and storage cap simultaneously.

Implementation landed in story 4.23 + close-out hardening iter-1:
- `V012__phase4_curator_retention.sql` adds `curator_decisions_summary`
  (per-week per-decision-type histogram + mean confidence, UNIQUE bucket
  for UPSERT idempotency).
- `crates/seasoned-hand-core/src/curator/retention.rs` ships the
  `CuratorRetentionJob` (90-day hot window, 60-day accelerated window when
  the SQLite footprint exceeds the 300 MB cap, atomic per-batch commit
  per NFR-4.3) and the `RetentionScheduler` daily-tick wrapper that
  `main.rs` spawns alongside the curator worker.

Deferred debt:
- Per-project retention classes (option C) later.
- Cron-expression scheduling (story 4.23 PM iter-1 wording was relaxed
  to `SH_CURATOR_RETENTION_INTERVAL_SEC` in close-out reconciliation;
  cron grammar can land in Phase 5 if ops demands non-uniform cadence).
- Cap-exceeded push trigger from other Curator components: deferred per
  REVIEW iter-2 — the daily-tick self-detect is already cap-correcting,
  external push is a precision optimization.

### 12.13 Q13 Embedding warm-up policy

Chosen option: **C (hybrid budget-aware warm-up)**.

Rationale:
- Smooths latency without violating NFR-4.6.

Deferred debt:
- Warm-up schedule tuner.

### 12.14 Q14 L2 cross-source verification rollout timing

Chosen option: **B (feature-flagged phased rollout by artifact class)**.

Rationale:
- Reduces rollout risk while landing enforcement in Phase 4.

Deferred debt:
- Global mandatory enforcement date to be set in Phase 4 close-out.

### 12.15 Q15 Knowledge/Datasource emit conditions

Chosen option: **C (two-tier emit: raw staging + promoted canonical)**.

Rationale:
- Preserves evidence capture while controlling active corpus quality.

Exact emit conditions:
- Knowledge raw staging emits when extraction has:
  - verdict PASS,
  - quality floor pass,
  - confidence >= 0.55,
  - at least one cited evidence reference.
- Datasource raw staging emits for each distinct source reference in extracted artifact with
  confidence >= 0.50.
- Promotion to canonical requires L2 corroboration:
  - either two independent source refs across distinct task ids,
  - or one source ref + one conflict-free matching revision evidence in same project.

Deferred debt:
- Threshold auto-tuning remains future work.

### 12.16 Q16 Weekly retrospective evidence policy

Chosen option: **C (hybrid template + analysis appendix)**.

Rationale:
- Strong auditability with room for useful synthesis.

Citation structure:
- Each claim sentence has `[[CIT:<kind>:<id>]]` tags, e.g.
  `[[CIT:event:12345]]`, `[[CIT:decision:cur_dec_...]]`.
- Coverage = cited_claims / total_claims; acceptance requires >=0.95.

Deferred debt:
- None.

### 12.17 Q17 Curator decision explainability format

Chosen option: **C (structured rationale object)**.

Rationale:
- Best long-term traceability and UI readiness.

Rationale JSON contract:
- `{"policy_version":"...","signals":{"fts":...,"embed":...,"struct":...},"thresholds":...,"explanation":"..."}`

Deferred debt:
- Rationale schema version migration tooling (DEBT #96).

### 12.18 Q18 Cross-project curation in Phase 4

Chosen option: **A (strict project isolation only)**.

Rationale:
- Aligns with Phase 5 boundary and avoids leakage risk.

Deferred debt:
- Optional read-only cross-project analytics can be reconsidered in Phase 5 (DEBT #95).

### 12.19 Q19 Curator governance for low-confidence actions

Chosen option: **C (sampled review with confidence bands)**.

Rationale:
- Scalable oversight without overwhelming humans.
- High-risk decisions still forced into 100% review.

Policy pin:
- Confidence < 0.55 -> always queue review.
- 0.55..0.75 -> 30% sampled review.
- >0.75 -> no default review unless high-impact action type.

Deferred debt:
- Sampling rates may be retuned by operational evidence.

### 12.20 Q20 Phase 4 close-out metric strategy

Chosen option: **B (CI replay + canary telemetry slice)**.

Rationale:
- Preserves reproducible gate while adding real-world sanity check.

Deferred debt:
- Full production auto-gate automation beyond canary remains future work.

## 13. Requirements coverage map

### 13.1 Functional coverage

| Requirement | Architecture sections |
|---|---|
| F-4.1 | §2.1, §6.2, §10 |
| F-4.2 | §2.1, §7, §12.4 |
| F-4.3 | §3.2 (playbook_revision_outcomes), §12.10 |
| F-4.4 | §2.2, §4.2, §7 |
| F-4.5 | §2.3, §4.2, §7, §12.2 |
| F-4.6 | §2.4, §4.3, §12.10 |
| F-4.7 | §3.2 (playbook_revisions), §12.10 |
| F-4.8 | §2.4, §3.2 (playbooks archive columns), §12.1, §12.11 |
| F-4.9 | §2.4, §3.2 (change_kind=restore), §10.3 |
| F-4.10 | §2.5, §4.4, §12.9 |
| F-4.11 | §2.5, §3.2 (sop_conflicts severity/status), §4.6 |
| F-4.12 | §3.2 (knowledge_items/datasource_items), §12.15 |
| F-4.13 | §12.14, §12.15, §11.2 |
| F-4.14 | §3.2 (source_project_id), §3.4, §10 |
| F-4.15 | §3.2 (curator_decisions), §4.5 |
| F-4.16 | §4.5, §6.4, §9 |
| F-4.17 | §2.6, §3.2 (weekly_retrospectives), §12.6, §12.16 |
| F-4.18 | §2.6, §8, §12.16 |
| F-4.19 | §2.7, §12.8 |
| F-4.20 | §2.7, §4.3, §12.10 |
| F-4.21 | §7, §11.3, §12.20 |
| F-4.22 | §8 |
| F-4.23 | §4.1, §6.5, §11.1 |
| F-4.24 | §9, §12.18 |
| F-4.25 | §2.8, §3.2 (curator_review_queue), §4.6 |
| F-4.26 | §10, §13 |

### 13.2 NFR coverage

| Requirement | Architecture sections |
|---|---|
| NFR-4.1 | §7, §8 |
| NFR-4.2 | §1, §6.2, §7 |
| NFR-4.3 | §3.2, §10 |
| NFR-4.4 | §3.2, §12.12 |
| NFR-4.5 | §2.5, §3.2 (sop_conflicts), §11 |
| NFR-4.6 | §2.3, §7, §12.13 |
| NFR-4.7 | §2.4, §9, §11.3, §12.2 |
| NFR-4.8 | §2.6, §11.3, §12.6, §12.16 |

### 13.3 Load-bearing implementation note

To prevent a repeat of the Phase 3 pre-story-3.17 structural gap, every load-bearing Phase 4
interface above has a corresponding runtime owner and persistence surface pinned in this document:
- runtime owner: CuratorWorker (§2.1)
- persistence: V011 schema (§3)
- integration path: existing components (§6)
- failure behavior: containment rules (§8)
- test plan: explicit gates (§11)

No interface in §4 is intended to remain test-only scaffold.
