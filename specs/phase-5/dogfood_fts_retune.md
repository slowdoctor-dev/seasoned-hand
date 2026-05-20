# Dogfood FTS5 retune procedure — Phase 5 story 5.24 / DEBT #76

> **Status**: procedure documented, full retune deferred to Phase 6 (DEBT #76 partial close).
> **Owners**: whoever runs the Phase 6 dogfood evaluation.

## Why this document exists

Story 5.24 promised a retune of FTS5 column weights for `playbooks_fts`,
`session_search_fts`, and `curator_search_fts` using "Phase 4 dogfood
data captured in the warm-loop benchmark." On audit, the warm-loop
benchmark's synthetic corpus is **insufficient** to converge a
meaningful weight tuple — synthetic queries don't exercise the
title/content/keyword balance the way real operator queries do. Per
the story's acceptance carve-out, this counts as a **partial close**:
the measurement procedure is documented here so the full retune can
land cleanly in Phase 6 once production dogfood exists.

## Eval set requirements

A meaningful retune needs at least **50 distinct operator queries**
hitting each of the three indices, with explicit relevance labels
(top-k rows the operator marked as "yes, this is what I wanted").
Capture protocol:

1. **playbooks_fts**: every `matcher::production_match` call against a
   live repository over a 2-week window. Log `(query_text, returned_ids,
   operator_action: accept | reject | edit_then_use)` to a `.jsonl`
   under `data/dogfood/playbooks/`.
2. **session_search_fts**: every CLI / HTTP search-session call. Log
   `(query_text, returned_event_ids, time_to_first_useful_hit_seconds)`
   to `data/dogfood/session_search/`.
3. **curator_search_fts**: every operator review-queue navigation in
   the curator admin UI. Log `(query_text, decisions_seen,
   decisions_acted_on)` to `data/dogfood/curator_search/`.

Anonymization: strip tenant_id + actor_user_id at capture time so the
corpus is portable across deployments. Keep the schema in
`specs/phase-6/dogfood_corpus_schema.md` (Phase 6 will add this).

## Relevance metric

**Primary**: precision@3 — fraction of top-3 results the operator
accepted (matcher) or marked as "useful" (search). Same metric as the
Phase 4 warm-loop benchmark's gate.

**Secondary**: MRR (mean reciprocal rank of the first accepted
result). Catches cases where precision@3 is stable but the top result
moved down the list, which still costs operator time.

## Weight search procedure

1. Hold out 20% of the eval set as a test fold; retune on 80%.
2. Grid-search across `(w_title, w_keywords, w_content) ∈
   {0.5, 1.0, 2.0, 3.0, 5.0, 8.0}³` for `playbooks_fts`. That's 216
   candidate tuples — under 30 seconds to evaluate against an eval set
   of 50 queries on a baseline laptop. Pick the tuple that maximizes
   precision@3 on the train fold, then verify on the test fold.
3. Same grid shape for `session_search_fts` over (event_type, source,
   searchable_text) and `curator_search_fts` over (decision_kind,
   rationale, raw_payload).
4. **Acceptance gate**: retune lands only if precision@3 improves by
   ≥3 percentage points on the test fold without regressing the
   Phase 4 warm-loop benchmark's precision@3 floor (story 4.21).

## Landing the new weights

The named constants in `crates/seasoned-hand-core/src/search/fts_weights.rs`
are the single source of truth. After the retune:

1. Update the `UNIFORM` constant on each weights struct (rename to
   `TUNED_PHASE6` or add a second associated const so the diff is
   reviewable).
2. Switch the matcher / session_search / curator_search MATCH queries
   from the default rank to `bm25(table, w1, w2, w3)` with the new
   tuple.
3. Rerun the warm-loop benchmark + the dogfood eval set; commit both
   numbers.
4. Update `specs/phase-5/DEBT.md` #76 from PARTIAL CLOSED to CLOSED
   with the retune evidence.

## Prior (best architectural guess, NOT validated)

If Phase 6 needs a starting prior before running the full grid search,
my guess on the strongest signal columns:

- **playbooks_fts**: `(title, keywords, content) = (5.0, 2.0, 1.0)` —
  authors hand-pick titles and keywords; content is the noisier
  signal because it carries narrative as well as triggering phrases.
- **session_search_fts**: `(event_type, source, searchable_text) =
  (0.5, 0.5, 1.0)` — operators usually search by content, not by
  event_type; the type+source are coarse filters not relevance signal.
- **curator_search_fts**: `(decision_kind, rationale, raw_payload) =
  (3.0, 2.0, 1.0)` — decision_kind is the structured handle, rationale
  is operator-authored prose, raw_payload is full event data.

These priors are **explicit guesses** — do not land them without
running the eval. They're written down so Phase 6 has a starting
point, not so Phase 5 can hand-wave a retune.

## Successor pointer

Phase 6 ROADMAP entry: "FTS5 weight retune with dogfood corpus" — to
be added when Phase 6 architecture lands. This document is the
hand-off artifact.
