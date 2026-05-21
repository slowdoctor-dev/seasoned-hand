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
