# Phase 3 — Open Questions

> Things the BMAD Architect (and Analyst, before them) will need to
> decide. **NOT pre-decided by this file** — each entry lists options
> with neutral pros/cons, not a recommendation. The Analyst sharpens /
> drops / merges these in `/specs/phase-3/requirements.md`; the
> Architect resolves the survivors in `/specs/phase-3/architecture.md`.
>
> Source signals: gathered cross-phase pre-Phase-3 review
> (`/specs/REVIEW.md` 2026-05-16) + ARCHITECTURE.md v1.1 reading +
> V009 schema audit + Phase 2 DEBT entries pointing to Phase 3.
>
> **Discipline note**: per AGENTS.md §11 "When stuck", every entry
> below states 2-3 options. The Architect does not pick blind.

---

## 1. Playbook schema reconciliation: ARCH §2.5 (rich) vs V009 (minimal)

**Context**: ARCHITECTURE.md v1.1 §2.5 spec's `playbooks` with:
- `trigger_keywords TEXT NOT NULL` (JSON array)
- `content TEXT NOT NULL` (inline content)
- `version INTEGER NOT NULL`
- `success_count INTEGER DEFAULT 0`
- `failure_count INTEGER DEFAULT 0`
- `avg_duration_ms INTEGER`
- `avg_tool_calls INTEGER`
- `status TEXT NOT NULL` (active / archived / pinned)
- `playbooks_fts` FTS5 virtual table

V009 shipped only `id, tenant_id, title, content_path, schema_version,
source_task_id, created_at, updated_at`. No `playbooks_fts`. No
`sops` table. No `glossary` table.

### Options

**A.** Ship `V010__phase3_learning_artifacts.sql` to extend `playbooks`
with the missing columns + create `playbooks_fts` + `sops` + `glossary`
exactly as ARCH §2.5 spec's.
- ✅ Code matches spec; no further ARCH drift
- ❌ Wide migration; some columns (`avg_duration_ms`, `failure_count`)
  may be premature without real usage data

**B.** Update ARCH §2.5 (v1.1 → v1.2 via a successor ADR) to match
V009's minimal shape; defer counters / status to Phase 4 Curator.
- ✅ Smaller surface, faster Phase 3
- ❌ Another version bump; risks "ship the easiest schema, not the
  right one"

**C.** Hybrid: V010 adds only `trigger_keywords` + `content` +
`playbooks_fts` (the matching-critical pieces); counters land in Phase 4.
- ✅ Matching path works immediately; counters arrive with the Curator
  that consumes them
- ❌ Two migrations on the same table within 2 phases

### Why this is #1
Everything else depends on what `playbooks` actually looks like.

---

## 2. Playbook content storage: inline TEXT vs `content_path` (file ref)

**Context**: ARCH §2.5 uses `content TEXT NOT NULL`; V009 uses
`content_path TEXT NOT NULL`. The Phase 2 provenance spill pattern
(100 KB inline → file ref) is precedent for going either way.

### Options

**A.** Inline (`content TEXT`) per ARCH §2.5
- ✅ One-query playbook read; no second I/O
- ❌ Large playbooks bloat row size; FTS5 over inline text is direct

**B.** File-ref (`content_path TEXT`) per V009
- ✅ Bigger playbooks don't bloat rows; matches the
  `provenance_manifest` spill pattern
- ❌ Two reads per playbook fetch; FTS5 needs the content somewhere
  (either index a denormalized column or maintain a `content` mirror)

**C.** Hybrid: small playbooks inline, spill above N KB
- ✅ Best of both
- ❌ More code; bigger test matrix

---

## 3. Playbook trigger matching: what algorithm?

**Context**: ROADMAP says "Playbook matching (new task → similar
playbooks)". ARCH §2.5 `trigger_keywords` is JSON-array-shaped, but
the matching algorithm isn't specified.

### Options

**A.** FTS5 over `trigger_keywords` + `title` + `content`
- ✅ SQLite-native; uses existing FTS5 infrastructure; transparent
- ❌ Keyword-shaped; can miss semantic matches ("refund a customer" vs
  "process return")

**B.** Embedding similarity (uses the reserved `embedding` model slot)
- ✅ Captures semantic similarity; future-proof for natural-language
  task descriptions
- ❌ Requires embedding-model setup; warm-cache cost on every match;
  ADR-003 12-slot model routing only has the `embedding` slot reserved,
  not wired

**C.** Hybrid: FTS5 first-pass + embedding rerank of top-N
- ✅ Cheap broad recall + semantic precision
- ❌ Most complex; two systems to maintain

---

## 4. Extraction trigger model: sync at task-complete vs async worker

**Context**: ADR-007 lists the 4 extraction criteria but doesn't pin
WHEN extraction runs. The Verifier worker (Phase 1 1.9b) is the closest
precedent — it consumes a Redis stream.

### Options

**A.** Sync at task-complete: as part of `task_complete` handler, run
extraction inline before returning
- ✅ Simple; no new worker; immediate availability
- ❌ Task-completion latency includes extraction cost (LLM call to
  draft playbook + DB write); slow user-facing close

**B.** Async via Redis stream worker (mirror Verifier pattern)
- ✅ Decouples completion from extraction; same XREADGROUP pattern
  Phase 1 1.9b uses; PEL retention for crash safety
- ❌ Another consumer-group surface to operate; double the Redis ops

**C.** Cron-style sweep (every N minutes, scan recent completions)
- ✅ Cheapest in steady state; batchable
- ❌ Latency between completion and playbook availability (the second
  run of the same task type may miss the just-extracted playbook);
  cron is the Curator's territory per ROADMAP §Phase 4

---

## 5. "Similar past tasks ≥ 2" — what counts as similar?

**Context**: ADR-007 criterion 3 says "≥2 similar past tasks exist
(pattern stability, not one-off)". `similar` is not defined anywhere.

### Options

**A.** Same project: count completed tasks under the same `project_id`
- ✅ Trivial to query; uses existing schema
- ❌ Over-aggregates (every Inbox task is in the same project) and
  under-aggregates across projects

**B.** Title/brief similarity: FTS5 over `tasks.title` +
`briefs.goal` against the new task's title/brief
- ✅ Semantic-ish without embeddings
- ❌ False positives on common words ("update", "fix")

**C.** Same `Brief.deliverable_format` + same task-type tag
- ✅ Structured; deterministic
- ❌ Requires a task-type taxonomy that doesn't exist yet

**D.** Defer: ship extraction without criterion 3 in Phase 3; add the
"≥2 similar" gate in Phase 4 once a corpus exists
- ✅ Smallest Phase 3 surface
- ❌ More aggressive than ADR-007 says; may pollute the playbook table

---

## 6. L2 cross-source verification — Phase 3 or Phase 4?

**Context**: ARCHITECTURE.md §6 spec's 4-layer verification. L1 (post-tool
hook), L3 (observation analysis), L4 (Verifier slot) all wired. L2
(cross-source) has no implementation — REVIEW §3 Section B noted this.

Phase 3's `Knowledge` event (= "fact established by ≥2 sources") is the
natural carrier for L2 enforcement.

### Options

**A.** Ship L2 in Phase 3 alongside `Knowledge` event emit
- ✅ Closes a long-known gap; gives playbooks a cleaner evidence trail
- ❌ Widens Phase 3 scope from "learning" to "verification + learning"

**B.** Stay tight: ship `Knowledge` event emit but no L2 enforcement
gate. Phase 4 Curator gates `Knowledge` retroactively.
- ✅ Smaller Phase 3
- ❌ Phase 3 playbooks may cite single-source `Knowledge` events

**C.** Don't emit `Knowledge` in Phase 3 at all (defer to Phase 4)
- ✅ Smallest surface
- ❌ Leaves the spec'd EventType variant still un-emitted; Phase 2
  DEBT #61 stays open

---

## 7. SOP authoring + storage surface

**Context**: SOPs are explicit, version-controlled, human-authored
(per ADR-007 + ARCH §2.5 `enforced BOOLEAN DEFAULT 1`). No authoring
UX exists today.

### Options

**A.** CLI-only: `seasoned-hand sop {create, edit, list, archive}`
subcommands write to V010 `sops` table; FE shows read-only listing
- ✅ Matches Phase 2 CLI surface; minimal FE work
- ❌ Power-user UX only

**B.** FE-first: dedicated SOP editor pane in the frontend
- ✅ Matches the "digital employee" framing (operator briefs the
  agent the way a manager writes onboarding docs)
- ❌ Frontend cost; Phase 3 is supposed to be backend-heavy

**C.** File-based: SOPs live in `~/.seasoned-hand/sops/*.md`,
content-addressed; the table mirrors filesystem state
- ✅ Operator can version SOPs in their own git repo; backups trivial
- ❌ Sync drift between FS and DB; harder multi-user later

**D.** Defer the authoring surface entirely: ship the `sops` table
+ `sop_read` real implementation; authoring is "SQL INSERT for now"
- ✅ Smallest Phase 3
- ❌ Phase 3 acceptance criterion ("second run faster") doesn't need
  authoring UX, but lack of one undermines the "explicit rules"
  half of the 4-layer model

---

## 8. Playbook injection: how many, where, at what token cost?

**Context**: ROADMAP says "playbook injection at task start (Initializer
context)". ARCH doesn't specify ceiling.

### Options

**A.** Top-1 match, injected as system message in Initializer
- ✅ Minimal token cost; simplest UX
- ❌ Worse-case: top-1 is wrong and the agent ignores or follows blindly

**B.** Top-N (e.g. 3) injected as system messages, all visible
- ✅ Agent can reason across multiple precedents
- ❌ Token cost; may dilute attention if some matches are weak

**C.** Top-N summarized into a single block via Initializer's planner
slot
- ✅ Capped token cost; agent gets the synthesis
- ❌ Adds a planner-LLM call to task start; potential latency

---

## 9. Session search — FTS5 over what?

**Context**: ROADMAP says "Session search via FTS5 + LLM summarization".
The `events` table has all session data, but FTS5-indexing it directly
is expensive (events.data is JSON; FTS5 wants tokenizable text).

### Options

**A.** FTS5 over `events.data` JSON contents directly
- ✅ One table, no denormalization
- ❌ JSON inside the FTS5 index is noisy (field names, syntax); needs
  a custom tokenizer; storage cost

**B.** Denormalize per-session search rows
(`session_search_index(session_id, snippet TEXT, role TEXT)`) with FTS5
- ✅ Clean search; cheap to query
- ❌ Synchronization (event-stream append → search index update);
  storage doubling

**C.** Index only Action+Observation+Misc (skip Plan/Knowledge etc.)
into FTS5
- ✅ Focuses search on user-visible content
- ❌ Loses Plan/Knowledge searchability

---

## 10. Acceptance criterion measurement: what task type, how measured?

**Context**: ROADMAP §Phase 3 acceptance: "A type of task, on the second
run, completes with 30% fewer tool calls." The Analyst must pin the
specific task type and measurement methodology.

### Options

**A.** Use an existing Phase 1 GAIA test as the benchmark; track
`sessions.tool_calls` delta between run 1 (cold) and run 2 (with
playbook)
- ✅ Reusable existing infra; deterministic
- ❌ GAIA tests aren't representative of real "employee" tasks

**B.** Synthesize a new "Phase 3 benchmark suite": a small set of
task templates (e.g., "summarize this PDF", "extract CSV from a web
page") run cold then warm
- ✅ Representative; specs the eval
- ❌ New eval infrastructure

**C.** Manual operator evaluation: dogfooding the system for a week
on real tasks; informal pass/fail
- ✅ Truest signal
- ❌ Not automatable; not story-completion gate-shaped

---

## 11. Curator scope boundary: where Phase 3 stops, where Phase 4 starts

**Context**: ROADMAP separates Phase 3 (learning starts) from Phase 4
(Curator + self-improvement). The boundary is fuzzy — playbook
extraction itself is curator-adjacent.

### Options

**A.** Phase 3 = "create + match + inject". Phase 4 = "rate, archive,
consolidate, retire". Hard line.
- ✅ Clear gates
- ❌ Phase 3 ships extracted playbooks that immediately need rating
  to be useful; phantom Curator dependency

**B.** Phase 3 = "create + match + inject + minimal feedback recording
(success_count++, failure_count++ on next-task verdict)". Phase 4 =
"automated quality decisions on top of the feedback record".
- ✅ Recording is cheap; gates the Phase 4 automation cleanly
- ❌ Phase 3 schema must include the counter columns (impacts #1)

**C.** Phase 3 ships everything except "auto-archive" (the Phase 4
delete decision). All extraction + matching + recording stays here.
- ✅ Tight Phase 4 (one feature)
- ❌ Phase 3 widens

---

## 12. Knowledge / Datasource / Skill event types — what triggers each?

**Context**: Phase 2 DEBT #61. ARCH §2.1 lists all three but doesn't
define the emit conditions.

### Options

**A.** Conservative emit rules:
- `Knowledge` = result of cross-source-verified fact lookup (L2)
- `Datasource` = explicit `info_search_web` / web_extract result
- `Skill` = playbook match at task start
- ✅ Each event has a single, testable emit site
- ❌ Phase 3 must also wire L2 (see #6)

**B.** Permissive emit:
- `Knowledge` = any `info_search_web` result (single source)
- `Datasource` = any URL the agent consulted
- `Skill` = any playbook fetched OR sop_read OR glossary_lookup
- ✅ Fills the events stream quickly; rich data for Phase 4 Curator
- ❌ "Knowledge" becomes a synonym for "search hit"; semantic dilution

**C.** Skip `Knowledge` and `Datasource` entirely in Phase 3 (only
`Skill` for playbook matches). Defer the other two to Phase 4.
- ✅ Smallest scope
- ❌ Reserved-but-unwired slots stay reserved-but-unwired

---

## How to use this list

1. **Analyst** (BMAD Analyst persona, fresh AI session): read this file
   + `INPUTS.md`. For each open question, decide:
   - Is it a real Phase 3 question? (drop if Phase 4 / Phase 5)
   - Is it pre-decided by an existing ADR? (drop if so, cite ADR)
   - What's the smallest answer that makes Phase 3 acceptance criteria
     met?
   Then write `/specs/phase-3/requirements.md`, resolving / merging
   questions inline. Update this file with "→ resolved in
   requirements.md §X" footers, do NOT delete entries.

2. **Architect** (BMAD Architect persona, fresh AI session): read
   `requirements.md` + this file (post-Analyst). Resolve remaining
   technical questions in `/specs/phase-3/architecture.md`. Do not
   re-open questions the Analyst already answered.

3. **PM** (BMAD PM persona, fresh AI session): read `requirements.md`
   + `architecture.md`. Break into 15-25 stories under
   `/specs/phase-3/stories/`. This file is informational only at
   the PM phase.
