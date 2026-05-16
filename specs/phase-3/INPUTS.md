# Phase 3 — Inputs

> One-page index of every spec, ADR, migration, code site, and external
> research note the BMAD Analyst will need to write
> `/specs/phase-3/requirements.md`. The Analyst's fresh AI session can
> open this file first and follow the links.
>
> **Status**: assembled 2026-05-16, pre-Analyst kickoff. Do not edit
> after the Analyst runs — instead, let `/specs/phase-3/requirements.md`
> supersede this index.

---

## 1. What Phase 3 ships (per existing roadmap)

`/specs/06-roadmap/ROADMAP.md` §Phase 3 — 4 weeks, "Learning system, learning starts":

- **4-layer learning data model**: SOPs, Playbooks, Project History,
  Glossary
- **Conservative learning trigger** per ADR-007
- **Playbook auto-extraction** from verified work
- **Playbook matching** (new task → similar playbooks)
- **Playbook injection** at task start (Initializer context)
- **Session search** via FTS5 + LLM summarization

**Acceptance**: a task type, on the second run, completes with **30%
fewer tool calls**.

---

## 2. Core philosophy + decision records

| File | Why it matters |
|---|---|
| `/AGENTS.md` §9 NEVER / §10 ALWAYS | Constraints the Analyst must honor |
| `/specs/00-philosophy/VISION.md` | The "time-axis benefit" claim Phase 3 makes real |
| `/specs/00-philosophy/PRINCIPLES.md` #4 (conservative learning), #15 (time is the agent's friend) | Decision filters when trade-offs surface |
| `/specs/00-philosophy/NON_GOALS.md` | What Phase 3 must NOT become (no marketplace, no fine-tuning, no general chat-memory store) |
| `/specs/01-architecture/decisions/ADR-007-conservative-learning.md` | The 4-criterion extraction policy: Verifier PASS + ≥5 tool calls + ≥2 similar past tasks + (optional) user-signaled satisfaction |
| `/specs/01-architecture/decisions/ADR-010-plan-as-process-control-block.md` | Plan structure that playbook injection augments at task start |
| `/specs/01-architecture/decisions/ADR-011-architecture-v1-1-text-drift-consolidation.md` | The v1.1 bump precedent — Phase 3 may need V1.2 if it adds new schema sections |

---

## 3. Immutable architecture surfaces Phase 3 must respect

| File / section | What's there | Phase 3 relevance |
|---|---|---|
| `/specs/01-architecture/ARCHITECTURE.md` v1.1 §2.5 | Spec'd SOPs / Playbooks / Glossary schemas (including FTS5 virtual table on `playbooks`, `trigger_keywords`, `success_count`, `failure_count`, `avg_duration_ms`, `status`) | **The richer schema in ARCH §2.5 does NOT match V009. See OPEN_QUESTIONS #1.** |
| ARCHITECTURE.md §6 | 4-layer verification framework. L2 (cross-source) currently has no implementation. | Phase 3 may want to ship L2 alongside playbook trust (`Knowledge` event = "fact established by ≥2 sources") — see OPEN_QUESTIONS #6 |
| ARCHITECTURE.md §2.1 + V002 | `EventType` 8 variants. `Knowledge`, `Datasource`, `Skill` are reserved + never emitted in production. | Phase 3 wires the emit sites. See Phase 2 DEBT #61. |
| ARCHITECTURE.md §3 12-slot model routing | `embedding` slot reserved but unused. | Playbook similarity / search may need it. See OPEN_QUESTIONS #4. |

---

## 4. Schema reality (what's actually in the DB)

| Migration | Tables / changes | Notes |
|---|---|---|
| `migrations/V009__phase2_skills_playbooks.sql` | `skills` (id, tenant_id, title, summary, schema_version, source_task_id, timestamps), `playbooks` (id, tenant_id, title, content_path, schema_version, source_task_id, timestamps), + tenant indexes | **Minimal schema only.** Missing vs ARCH §2.5: `playbooks_fts` FTS5 virtual table, `playbooks.trigger_keywords`, `playbooks.content` (V009 uses `content_path` instead), `playbooks.{success_count, failure_count, avg_duration_ms, status, version}`. `sops` and `glossary` tables don't exist yet. |
| `migrations/V006__phase2_projects_tasks.sql` | `tasks.id` is the natural `source_task_id` foreign key for playbook extraction | |
| All `events` rows (V002) | `Action`, `Observation`, `Plan`, `Misc` events are the raw learning corpus | Conservative extraction reads these post-task |
| `verifications` (V004) | `verdict ∈ {pass, fail}` is the L4 gate ADR-007 conditions extraction on | |
| `deliverables.provenance_manifest` (V007) | Full evidence trail per deliverable — input to playbook-extraction context | Phase 2 DEBT #5 may pressure compression |

---

## 5. Code surfaces that touch Phase 3

| File | Current state | Phase 3 work |
|---|---|---|
| `crates/seasoned-hand-core/src/tools/builtin.rs` `SopRead` / `PlaybookSearch` / `GlossaryLookup` | Stubs returning `{ok:false, error:"not_implemented", message:"deferred to phase 3"}` | Replace with real implementations against V010+ schema |
| `crates/seasoned-hand-core/src/skill/mod.rs` | `SkillStore` exists with reserved schema | Phase 3 first writer |
| `crates/seasoned-hand-core/src/events/mod.rs` `EventType::{Knowledge, Datasource, Skill}` | Enum variants exist + V002 CHECK accepts them; zero production emitters | Wire emit sites |
| `crates/seasoned-hand-core/src/agent/init/mod.rs` Initializer | Authors Brief; no playbook-injection step | Inject matched playbooks at task start |
| `crates/seasoned-hand-core/src/verifier/worker.rs` | Posts `verifier_verdict` Misc on **any parsed verdict** (both `pass` and `fail`); payload shape includes `verification_id` + `verdict` + `evidence_event_ids` (see `worker.rs:499-527`) | Extraction trigger reads the Misc but must filter `verdict == "pass"` per ADR-007 criterion 1. The event payload shape is the input contract for extraction. |
| `crates/seasoned-hand-core/src/router/capability/mod.rs` | Generic alias resolution exists at `:65-166` (Bifrost `/v1/models/:alias` lookup → static `capabilities_for` table); the `embedding` slot is registered but no Phase 3-specific embedding-search wiring exists | If Phase 3 picks embedding-based playbook similarity (see `OPEN_QUESTIONS.md` #3), wire the embedding slot through the slot router (`router::SlotRouter`) — not just the capability resolver. |
| `crates/seasoned-hand-core/src/router/mod.rs` `SlotRouter` | 12 slots resolvable; consult site for any new playbook-matching LLM call | Extraction pipeline + matching pipeline will need to resolve the model slot for their LLM calls — see `SlotRouter::resolve(SlotName::Planner)` precedent at `deliverable/task_deliver.rs:130`. |
| `crates/seasoned-hand-core/src/agent/prompt.rs` `build_messages` (`pub(crate)`) | Builds the per-iteration message list consumed by the agent loop | Playbook injection has two candidate insertion sites: (a) Initializer-only (one-shot at task start, in the system prompt) or (b) `build_messages` (sticky across every iteration, in the prompt header). Architect picks; this site is the alternative to (a). |
| `crates/seasoned-hand-server/src/lib.rs` HTTP surface (~3100 lines after the Codex-review-driven loopback gate additions) | No `/v1/playbooks`, `/v1/sops`, `/v1/glossary` routes; lib.rs is also tracked as DEBT #52 for a Phase 3 warm-up split | New routes for browsing / editing learning artifacts (UX + CLI). Should land alongside the lib.rs split if Phase 3 chooses to do both. |
| `crates/seasoned-hand-cli/src/commands/` | No `sop`, `playbook`, `glossary` subcommands | CLI surface for SOP authoring (see `OPEN_QUESTIONS.md` #7) |

---

## 6. External research (Manus / Hermes inputs)

| File | Key claims relevant to Phase 3 |
|---|---|
| `/specs/07-research/manus-direct-qa.md` | Manus's own wishlist explicitly includes "global knowledge graph" (≈ our Playbooks + Glossary) and "faster inner loop" (extraction speed matters). Quote: *"I am like a brilliant consultant who walks into your office every morning with total amnesia of yesterday's meeting."* — this is the gap Phase 3 closes. Also: 4-layer verification (L1-L4) detail, "cumulative state" framing for Event-Stream replay, sequential-tool reasoning for cascading-error prevention. |
| `/specs/07-research/manus-map-tool-spec.md` | Map tool deferred to Phase 4+ per ADR-009 — not Phase 3 scope |
| `/specs/07-research/manus-plan-tool-spec.md` | Plan-tool spec — already implemented in Phase 1; informational only |

---

## 7. DEBT entries that name Phase 3 as pay-down

See `/specs/phase-3/DEBT.md` for the inheritance list. The two most
load-bearing for the Architect:

- **Phase 2 DEBT #6** — V009 reserved the tables; Phase 3 fills them
- **Phase 2 DEBT #61** — `EventType::Knowledge/Datasource/Skill`
  reserved variants need emit sites

The Phase 3 close should also re-evaluate **Phase 2 DEBT #7**
(Verifier rollback default opt-in) once a month of real verdict data
exists.

---

## 8. Open questions parked for the Architect

See `/specs/phase-3/OPEN_QUESTIONS.md`. They are NOT pre-decided by
this index — the Analyst should sharpen / drop / merge them while
writing requirements.md. The Architect reads the surviving set when
authoring architecture.md.

---

## 9. Cross-phase REVIEW context

`/specs/REVIEW.md` is the pre-Phase-3 cross-phase hardening review
(Claude pass). `/tmp/codex-review.md` is the follow-up Codex review of
that pass, which surfaced an additional 7 issues including the
ARCH-§2.4 arithmetic errata fixed in ADR-011's 2026-05-17 errata.
Findings closed by the hardening + Codex-follow-up commits
(`18d472d` through HEAD) are recorded in the relevant phase DEBT
ledgers — see `/specs/phase-3/DEBT.md` for the full Phase 3
inheritance list (§7 above only names the two most load-bearing).

The post-close hardening pattern Phase 2 set (REVIEW → hardening
commits → DEBT append) is the model Phase 3 should follow at its own
close.

---

## 10. What the BMAD Analyst's deliverable looks like

Per `/docs/methodology.md` + the Phase 0/1/2 precedent, the Analyst
produces `/specs/phase-3/requirements.md` containing:

1. Goal statement (one sentence, then one paragraph)
2. Acceptance criteria (measurable; ROADMAP says "30% fewer tool calls
   on second run of same task type" — Analyst pins what "same task
   type" means and how to measure)
3. In-scope features list
4. Explicit non-goals (what Phase 3 is NOT)
5. Resolved open questions (which of OPEN_QUESTIONS.md got answered;
   which got pushed to the Architect)
6. Risks + mitigations

Then the **BMAD Architect** picks up and writes
`/specs/phase-3/architecture.md` from `requirements.md` + this index.
Then the **BMAD PM** breaks the architecture into stories under
`/specs/phase-3/stories/`.

Each persona runs in a fresh AI session.
