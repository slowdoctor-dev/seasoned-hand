# Performance Review Log

Hardening track for **performance** — a Claude + Codex bilateral pass to
saturation (mirrors `specs/SECURITY_REVIEW.md` / `MAINTAINABILITY_REVIEW.md`).

**Bar for an "issue":** a CONCRETE inefficiency on a path that is actually hot
(per-event / per-agent-loop-iteration / per-request / per-row-in-a-loop), with a
behaviour-preserving, clearly-net-positive fix. **NOT** speculative
micro-optimization on cold/startup paths, NOT caching complexity without a real
repeated cost, NOT architectural rewrites (the single `Arc<Mutex<Connection>>`
SQLite pool is a known documented tradeoff — out of scope). Premature
optimization is itself a defect here.

**Saturation rule:** a bilateral round in which neither Claude nor Codex finds a
new actionable hot-path inefficiency, all prior items resolved, gates green
(`clippy --all-targets -D warnings` / `fmt --check` / `cargo test --workspace` /
`spec-check` 10/10).

---

## Audit cycle — 2026-05-21 (Claude + Codex)

### iter-1 (Claude) — audit + 2 hot-path fixes

The codebase was already well-tuned: PII-redactor regexes are `LazyLock`-cached
(not recompiled per call), the DB mutex is never held across `.await`
(`with_conn` takes a synchronous closure), hot reads are `LIMIT`-capped, all hot
WHERE/JOIN columns are indexed (events `idx_events_session_time`, playbooks
source/project/FTS indexes), config/verifier/narrator prompts load once at boot,
and the billing/retention batch inserts are wrapped in single transactions. Two
genuine hot-path issues found and fixed:

| # | Issue | Hot path | Fix | Risk |
|---|-------|----------|-----|------|
| P1 | `Initializer::new` read `config/prompts/planner.system.txt` synchronously every construction | a fresh `Initializer` is built **per task** inside `tokio::spawn` (`initializer_spawner.rs:115`) → blocking disk read on a Tokio worker + repeated I/O of a static file | cache via `static PLANNER_PROMPT: LazyLock<String>` (read once, process-wide; prompt is operator-static, matching the verifier/narrator boot-load) | Low |
| P2 | `tool_specs_from_registry()` rebuilt all ~38 tool JSON schemas (a fresh `json!` each) + sorted, **every agent-loop iteration** (`agent/mod.rs`, 50+×/task) | the tool catalogue + mask policy/mode are invariant for a run | hoist the masked `Vec<ToolSpec>` out of the loop (build once, `clone()` into each request) | Low |

**Leads checked and found FINE (no action):** verifier/narrator/notify/router
prompt reads (boot-only); lock-across-await (structurally impossible via
`with_conn`); `build_messages` per-iteration read (`LIMIT 100` + indexed);
curator `ConflictDetector` per-pair mutex (bounded background job, LLM-dominated);
N+1 (billing/retention batched in one tx); indexes (all hot columns covered).

Both fixes are behaviour-preserving (cached value / hoisted invariant — same
result each call). Committed `d5d5f77`.

### iter-2 (Codex) — committed `ef8202f` (2026-05-22)

Codex's independent pass found one hot-path allocation issue and fixed it:

- **P3 (Codex):** `SqliteEventStore`'s event-read query (`events/sqlite.rs`,
  the `WHERE session_id = ? [...] ORDER BY id ASC LIMIT ?` read used by
  `build_messages` **every agent-loop iteration**) built its bind params as a
  `Vec<Box<dyn ToSql>>` + a `Vec<&dyn ToSql>` — heap-boxing each param + two
  vecs per call. Codex replaced it with a `match` over the 4 (after_id ×
  type_filter) combinations using stack `rusqlite::params![]`, removing the
  per-call allocations. Behaviour-preserving (same SQL, params, rows).

P3 verified behaviour-preserving by Claude (read the diff: same SQL, params,
rows); Codex's gates green; committed + pushed as `ef8202f` (only
`events/sqlite.rs`). **Next session:** run the independent **iter-3** confirm
(Claude sweep + Codex confirm). Saturation = a bilateral round where neither
party finds a new hot-path issue. Tracks sealed so far this cycle: Security,
manageability; performance has 3 fixes (P1/P2/P3) and awaits its iter-3 seal.

### iter-3 (Claude) — confirm sweep, no new issues

Independent confirm pass. First re-verified the three landed fixes are
actually in the tree and behaviour-preserving:

- **P1** present: `agent/init/mod.rs:719` `static PLANNER_PROMPT: LazyLock<String>`,
  consumed at `:90` via `.clone()`; falls back to `DEFAULT_PLANNER_PROMPT`.
  Read once process-wide, no per-task disk I/O. ✓
- **P2** present: `agent/mod.rs:233-237` builds `masked_tools` once *before*
  the `for step in start_step..req.max_steps` loop (`:239`); the loop body
  only `masked_tools.clone()`s into each request (`:277`). No per-iteration
  schema rebuild. ✓
- **P3** present: `events/sqlite.rs:211-223` is a 4-arm `match` over
  `(after_id, type_filter)` using stack `rusqlite::params![]`; no
  `Box<dyn ToSql>`, no intermediate param vecs. ✓

Fresh sweep of hot paths not previously sealed — none actionable:

- **`build_messages`** (`agent/prompt.rs:14`, per agent-loop iteration): one
  `LIMIT 100` query on the indexed `(session_id, id)` path + a single linear
  pass building `Vec<Message>`. Rebuilding context from the event log each
  iteration is the ReAct design ("context = RAM"), not a defect. Bounded. FINE.
- **`SqliteEventStore::append`** (`events/sqlite.rs:61`, per-event write): one
  indexed `SELECT 1 FROM sessions` FK guard + one `INSERT … RETURNING` +
  projection/search hooks, all in a single `with_conn` transaction. The guard
  is a correctness check dwarfed by the INSERT; no per-row work. FINE.
- **`masked_tools.clone()` per iteration** (`agent/mod.rs:277`): a ~38-element
  `Vec<ToolSpec>` deep-clone each step. Considered wrapping in `Arc`, but the
  same iteration issues a multi-second LLM round-trip — the clone is orders of
  magnitude below the dominant cost. Per this log's own bar ("premature
  optimization is itself a defect"), NOT actionable. FINE.

Gates this pass: `clippy --all-targets -D warnings` ✓, `fmt --check` ✓,
`spec-check` 10/10 ✓. `cargo test`: all non-sandbox tests pass (core 421
pass); the only failures are Docker-socket-dependent suites that cannot run in
a daemon-less environment (not code defects) — re-run on a Docker host to seal
the test gate. Claude half
of iter-3 is **clean — no new hot-path issue**. Per the saturation rule, the
performance track seals once the **Codex** confirm half of iter-3 also comes
back clean (a bilateral round with zero new findings). Until then: **3 fixes
landed, Claude-confirmed, awaiting Codex confirm to seal.**

### iter-3 (Codex) — confirm sweep, ONE new issue (P4)

Codex's confirm half did **not** come back clean: it drilled into the
write-time hooks the Claude half had waved past (the iter-3 Claude note marked
`SqliteEventStore::append` "FINE" but only examined the FK guard + INSERT, not
the projection/search hooks chained after it). Genuine per-event hot-path find:

- **P4 (Codex):** on every successful `append`, `visibility::apply` computes the
  projection `(tenant_id, visibility_level, searchable_text)` and INSERTs the
  `tenant_event_view` row — then `session_search::index_event_for_search`
  immediately ran a **second** `SELECT tenant_id, visibility_level,
  searchable_text FROM tenant_event_view WHERE event_id = ?` to read those exact
  values back before inserting the search-index row. A redundant per-event query
  + row decode on the append path, with the values already in hand.

  **Fix:** `ProjectionOutcome::Inserted` now carries a `SearchProjection
  { tenant_id, visibility_level, searchable_text }` payload (the values
  `apply` already materialized — `params!` only borrowed them, so they're moved
  out after the INSERT at no extra cost). `index_event_for_search` takes those
  values as args and drops the `SELECT`. Behaviour-preserving: the threaded
  values are byte-identical to what the view stored (same `visibility_for`,
  same `resolve_tenant_id`, same post-`redact_pii` `searchable_text`); search
  indexing still happens only on `Inserted` (Story 5.15 invariant now upheld by
  the caller's `if let Inserted`, not a defensive re-read). Touches
  `events/visibility.rs`, `events/session_search.rs`, `events/sqlite.rs`.

Gates: `clippy -p seasoned-hand-core --all-targets -D warnings` ✓, `fmt --check`
✓, `cargo test -p seasoned-hand-core --lib` **599 passed / 0 failed / 13
ignored** (the ignored are the Docker-socket suites). Claude verified the diff
is behaviour-preserving.

**Seal status:** iter-3 was NOT a clean bilateral round (Codex found P4), so the
track is **still unsealed** — now **4 fixes landed (P1–P4)**. Sealing requires
an **iter-4** bilateral round where *both* halves sweep and *neither* finds a
new hot-path issue. Next session: run iter-4 (Claude sweep + Codex confirm) over
the same hot paths plus the now-simplified append/index path.

### iter-4 (Claude + Codex) — clean bilateral round → **TRACK SEALED**

Both halves swept independently after P4 landed; neither found a new actionable
hot-path inefficiency.

- **Claude half — clean.** Append path is optimal post-P4 (the redundant
  per-event re-`SELECT` is gone; remaining per-event work is single indexed
  statements). `build_messages` (`agent/prompt.rs`) is bounded `LIMIT 100` on
  the indexed `(session_id, id)` path. Per-request auth `verify`
  (`auth/session.rs:157`) is a single indexed 4-table join on `s.token_hash` —
  a deliberate live-identity re-resolution (ADR-018), no N+1. The per-event
  `resolve_tenant_id` join (`events/visibility.rs`) repeats a cheap indexed
  sessions→tasks→projects lookup whose result is invariant per session, but
  caching it would add session-scoped state + invalidation risk for an indexed
  PK join — judged NOT actionable per the bar.
- **Codex half — clean (confirm).** Independent sweep (incl. re-reading the
  search-index path and the `sessions`/`tenant_id`/`session_search_index`
  migrations). Explicitly concurred on `resolve_tenant_id` ("a deliberate
  derived-tenant cost, not a clear net-positive cache win") and dismissed the
  remaining request-side param-bind boxing as "too small relative to the DB/FTS
  work to justify a finding." Verdict: **confirm clean (seal).**

**PERFORMANCE TRACK SEALED** — 4 fixes total (P1 planner-prompt LazyLock, P2
hoisted masked tool specs, P3 stack `rusqlite::params!` on the event read, P4
projection-value reuse on the append/search-index hook). Saturation reached: a
bilateral round (iter-4) with zero new findings. Remaining test-gate caveat:
the Docker-socket-dependent suites still need one `cargo test --workspace` run
on a real Docker host to seal the *test* gate (orthogonal to this perf seal;
tracked in issue #6's release checklist).
