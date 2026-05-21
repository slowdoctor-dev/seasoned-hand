# Maintainability Review Log

Hardening track for **manageability, simplicity, and code-reusability** — a
behaviour-preserving refactor loop run as a Claude + Codex bilateral pass to
saturation (mirrors `specs/SECURITY_REVIEW.md`).

**Bar for an "issue":** a concrete, behaviour-preserving defect — *real*
duplication (same logic in ≥2 sites), dead/unreachable code, a construct with a
strictly simpler equivalent, or a god-file/function whose split materially aids
navigation. NOT taste-only churn, and NOT speculative abstraction for
hypothetical reuse (the project's no-over-engineering rule still binds).

**Saturation rule:** a bilateral round in which neither Claude nor Codex finds a
new actionable defect, with all prior items resolved and all gates green
(`clippy --all-targets -D warnings` / `fmt --check` / `cargo test --workspace` /
`spec-check` 10/10).

---

## Audit cycle — 2026-05-21 (Claude + Codex)

### iter-1 (Claude) — audit + small consolidations

Survey: core ~53K LOC, server ~6.8K, cli ~3K. Largest files: `curator/mod.rs`
(6313), `server/src/lib.rs` (4274), `tools/builtin.rs` (1696),
`verifier/gate.rs` (1528), `ws.rs` (1194). Overall the codebase is well
disciplined — a shared `time.rs` already exists with an anti-duplication
rationale, there is no `#[allow(dead_code)]` or commented-out cruft, and
`curator` already uses sibling-file splits. Findings were a handful of small
copy-paste helpers, one mechanical boilerplate collapse, and one oversized file.

| # | Item | Category | Risk | Status |
|---|------|----------|------|--------|
| 1 | `truncate(s,n)` copied verbatim in `deliverable/task_deliver.rs`, `channel/ntfy.rs`, `sandbox/bootstrap.rs` | Duplication | Low | **fixed** → `crate::text::truncate` |
| 2 | `sha256_hex(bytes)` copied in `deliverable/task_deliver.rs` + `org/invitation.rs` | Duplication | Low | **fixed** → `crate::hash::sha256_hex` |
| 3 | `now_micros_for_rollback` (`tools/builtin.rs`) + `now_unix_micros` (`server/lib.rs`) re-impl the clock with a *wrapping* cast | Duplication | Low | **fixed** → `crate::time::now_micros` (also unifies overflow semantics, the reason `time.rs` exists) |
| 4 | `sandbox_get_raw`/`sandbox_post_raw` share a ~20-line response-mapping tail (`tools/builtin.rs`) | Duplication | Low | **fixed** → `map_sandbox_response` |
| 5 | ~116 inline `(StatusCode::X, Json(ApiError{error:"code".into()}))` tuples in `server/lib.rs` | Duplication/Complexity | Low | **assigned to Codex iter-2** (`api_err` constructor) |
| 6a | `curator/mod.rs` test module (~3036 lines) inline | God-file | Low | **fixed** — `mod tests` extracted to `curator/tests.rs` (mirrors sibling `tenant_boundaries_tests`); mod.rs 6313 → 3277 lines, 59 curator tests still green |
| 6b | `curator/mod.rs` production code ~3277 lines | God-file | Med | **assessed → deliberately DEFERRED** (see note) |

**Non-issues (checked + cleared, do not "fix"):** `SimpleLru` does not duplicate
a dependency (no `lru`/`hashlink` crate present — hand-roll is correct under
no-new-deps); `source_ext_for` vs `format_to_str` are semantically distinct;
the per-`Tool` `name/description/schema/invoke` impls are inherent trait
boilerplate; the per-call `map_err(|e| ToolError::Backend(..))` sites are
idiomatic, not worth a helper; the `Result`-returning `now_*` variants in
`curator`/`events`/`plan` carry distinct error types and must stay separate.

iter-1 fixed items 1–4 (small consolidations): new focused modules
`crate::text` (`truncate`) and `crate::hash` (`sha256_hex`) mirroring the
`time.rs` precedent; `map_sandbox_response` extracted. Behaviour-preserving;
gates green.

**Division of labour for the next round (no file overlap):** Codex takes item 5
(`server/lib.rs` only); Claude takes item 6 (`curator/*` only).

### iter-1b note — curator production split (6b) assessed, deferred

After extracting the test module (6a, done), the remaining production code is
~3277 lines. The audit proposed splitting it into `sqlite.rs` / `embedding.rs`
/ `helpers.rs`. On inspection the seams are **not** cleanly separable:
- the `Sqlite*` trait impls (the largest contiguous block, ~441–1547) call the
  scope validators (`validate_decision_scope`/`validate_revision_scope`/
  `project_tenant_id`, only used here) **and** ~8 scattered pure helpers defined
  far later in the file (`lexical_overlap` ×7, `stable_u64_hex` ×6,
  `cosine_similarity`, `structural_conflict_score`, `compose_confidence_with_bounds`,
  `infer_knowledge_key`, `apply_merge`, `review_required`);
- the pure helpers themselves are interleaved with async-DB fns and mutation
  logic, so there is no clean contiguous "helpers" block to lift.

A split would therefore require bumping ~8+ private fns to `pub(super)`, glob
`use super::*` imports, and a `pub use` re-export to preserve the public API —
churn across a critical learning subsystem for an incremental 3277→~2100-line
gain. Per the project's conservative / no-over-engineering rule, this is **not
worth the risk** now. **Disposition:** the categorical god-file problem (a 6313-
line file that was half tests) is resolved by 6a; the cohesive 3277-line curator
module is acceptable. Revisit only if the curator gains a genuinely independent
concern that forms a clean module on its own.

### iter-2 (Codex) — item 5: `api_err` constructor

Codex added `fn api_err(status: StatusCode, code: String) -> ApiErrorResponse`
(`server/lib.rs:832`) and replaced all 116 inline
`(StatusCode::X, Json(ApiError { error: ... }))` tuples with `api_err(...)`
calls (117 call sites; the single remaining `Json(ApiError {` is inside the
helper). Behaviour-preserving — the helper builds the identical tuple, same
status codes + error strings, full suite green. Net `server/lib.rs` −413 lines
(commit `5ab0240`). Verified by Claude against `git diff` (lib.rs only) + the
helper body + an independent gate run. Codex reported **0 new manageability
findings** beyond item 5.
