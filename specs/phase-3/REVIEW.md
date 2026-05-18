# Phase 3 — Cross-phase Hardening Review (Claude iter-1)

> Date: 2026-05-18
> Reviewer: Claude (iter-1 of alternating Claude/Codex hardening)
> Scope: all 16 stories (`6317a39..c3224ed`) + Phase 3 implementation footprint
> Pattern mirrors `/specs/REVIEW.md` pre-Phase-3 → DEBT append → Codex follow-up sequence
> (`324e946` → `35654f0` → `1bc0e4f`).

Methodology — actually ran the AGENTS.md §6 gates, deep-read the 4 hotspot files
(`verifier/gate.rs` 749Δ, `matcher/mod.rs` 576Δ, `verifier/extraction.rs` 301-line new,
`ARCHITECTURE.md` 78Δ), and spot-checked the security-critical paths against the
F-3.13 / F-3.14 contracts.

## Findings summary

| # | Severity | Category | Title |
|---|---|---|---|
| F1 | **H** | quality-gate | `cargo test --workspace` FAILS — 2 CLI tests race on `SH_DATABASE_URL` env var |
| F2 | **H** | quality-gate | `cargo fmt --check` FAILS — multiple files unformatted (story 3.10 + others) |
| F3 | **H** | quality-gate | `cargo clippy --all-targets -- -D warnings` FAILS — 6 lints |
| F4 | **M** | spec-fidelity | Gate matcher crams `fixture:X / brief:Y` sentinels into `trigger_keywords` — pollutes production FTS5 + LIKE-prefix false positives |
| F5 | **M** | security | F-3.14 `AUTH_HEADER_RE` only matches `Authorization: Bearer` + `X-Api-Key:` — misses Cookie / Set-Cookie / X-Auth-Token / Proxy-Authorization |
| F6 | **M** | security | F-3.14 `PHONE_RE` + `IPV4_RE` over-redact common false positives (software version strings like `1.2.3.4`, ID-shaped date tokens) |
| F7 | **M** | process | Story 3.16 acceptance close-out skipped 3 of the 6 AGENTS.md §6 verification gates — letting F1/F2/F3 ship to main |
| F8 | **L** | spec-fidelity | No orchestrator-level test for architecture §3 step 6 "post-cap quality_floor fail → emit BOTH events" combined path |

---

## F1 (H) — `cargo test --workspace` fails: env-var race in CLI tests

**Evidence**

```
running 3 tests
commands::playbook::tests::lifecycle_list_show_delete --- FAILED
commands::sop::tests::create_edit_list_show_delete_roundtrip --- FAILED

---- commands::playbook::tests::lifecycle_list_show_delete stdout ----
thread 'commands::playbook::tests::lifecycle_list_show_delete' panicked at
crates/seasoned-hand-cli/src/commands/playbook.rs:185:44:
called `Result::unwrap()` on an `Err` value:
  Sqlite(SqliteFailure(Error { code: DatabaseBusy, ... }, Some("database is locked")))

---- commands::sop::tests::create_edit_list_show_delete_roundtrip stdout ----
called `Result::unwrap()` on an `Err` value:
  migration error: `error asserting migrations table`, `disk I/O error`
```

**Root cause** — both `crates/seasoned-hand-cli/src/commands/{sop,playbook}.rs` test
modules use `unsafe { std::env::set_var("SH_DATABASE_URL", per_test_tmpdir) }` to
point each test at its own SQLite DB. But cargo's default test runner executes tests
in PARALLEL within the same binary process. The two tests race on the global
`SH_DATABASE_URL` — one test ends up reading the OTHER test's DB path, causing
DB-busy locks and migration-table races.

Codex's per-story verification command (`cargo test -p seasoned-hand-cli
commands::playbook`) only runs one test at a time, so the race never appears in
isolation. The race only fires under `cargo test --workspace` — which AGENTS.md §6
lists as a gate but story 3.16 close-out never ran.

**Recommended fix** — add `serial_test = "3"` to `seasoned-hand-cli/Cargo.toml`
`[dev-dependencies]` and slap `#[serial_test::serial(SH_DATABASE_URL)]` on both
tests. Alternative: refactor `run()` to accept `db_url: &str` instead of reading
env (cleaner long-term but a bigger surface change). Going with `serial_test` is
the minimal-blast-radius fix for Phase 3.

---

## F2 (H) — `cargo fmt --check` fails

**Evidence** — `cargo fmt --check` reports diffs in at least these files:

```
Diff in crates/seasoned-hand-cli/src/commands/playbook.rs:3
  (use ordering)
Diff in crates/seasoned-hand-cli/src/commands/playbook.rs:225
  (closure body line-wrap)
Diff in crates/seasoned-hand-cli/src/commands/playbook.rs:234
  (trailing blank line)
Diff in crates/seasoned-hand-cli/src/commands/session_search.rs:53
  ...
```

(Earlier check also flagged a block in `verifier/gate.rs::warm_benchmark`
`assert_eq!` line-wrap.)

**Root cause** — Codex committed CLI stories without running `cargo fmt` first.
AGENTS.md §6 lists `cargo fmt --check` as a gate; story-level verification skipped it.

**Recommended fix** — run `cargo fmt` once at repo root, commit the diff.

---

## F3 (H) — `cargo clippy --all-targets -- -D warnings` fails: 6 errors

**Evidence** (6 errors when `-D warnings` is honored):

1. `module has the same name as its containing module` ×3
   - `crates/seasoned-hand-core/src/verifier/extraction.rs:217` — `mod extraction { ... }` inside `extraction.rs`. Should be `mod tests { ... }`.
   - Two more occurrences (likely in `matcher/mod.rs:249` `mod matcher` and another).
2. `unnecessary use of get("X").is_some()` ×3 — tool-catalog assertion tests use
   `reg.get("sop_read").is_some()` etc. Should be `reg.contains_key("sop_read")`.

**Root cause** — same as F2: per-story verification ran `cargo test` without `--all-targets -- -D warnings` flag. Story 3.16's spec-check.sh hook validates spec-shape, not clippy gates.

**Recommended fix** — rename three `mod $name` test modules to `mod tests`; replace
three `get(...).is_some()` with `contains_key(...)`. ~10 line diff total.

---

## F4 (M) — Gate matcher fixture/brief sentinels leak into production FTS

**Evidence** (`crates/seasoned-hand-core/src/matcher/mod.rs:88-96`):

```rust
let fixture_key = format!("fixture:{fixture_id}");
let brief_key = format!("brief:{}", normalized_brief);
let mut stmt = conn.prepare(
    "SELECT ...
     FROM playbooks p
     ...
     WHERE ...
       AND lower(p.trigger_keywords) LIKE '%' || lower(?) || '%'
       AND lower(p.trigger_keywords) LIKE '%' || lower(?) || '%'",
)?;
```

And the fixture seed (`verifier/gate.rs:1334`):

```rust
format!("[\"fixture:{}\", \"brief:{}\"]", fixture.fixture_id, normalized);
```

**Issues**

1. **FTS5 pollution** — `playbooks_fts` indexes `trigger_keywords`. A production-mode
   query for `"phase2"` would tokenize the sentinel `fixture:phase2_overnight_default_path`
   and hit the gate-fixture playbook even when the operator just asked for a generic
   "phase2" playbook. The gate-only seed leaks into production-mode results.
2. **LIKE-prefix false positives** — `'%fixture:phase2_overnight_default_path%'`
   substring-matches *any* playbook whose `trigger_keywords` happens to contain that
   string anywhere — including future fixtures with the same prefix (e.g.
   `phase2_overnight_default_path_v2`). The gate-mode "strict identity" contract
   (F-3.4) is satisfied only by convention, not by the type system.
3. **Layering violation** — the architecture pin (§2.2 gate-mode identity = tuple
   `(fixture_id, normalized_brief)`) implies first-class data shape. The implementation
   encodes the tuple as substrings of a JSON-shaped TEXT column. Future schema changes
   to `trigger_keywords` (e.g. Phase 4 retuning) could silently break gate matching.

**Recommended fix** (Phase 3 hardening + DEBT)

- Short-term (Phase 3 hardening): leave the implementation but file DEBT seed
  documenting the smell + the test that would catch the FTS pollution: seed a
  benign playbook whose content contains `"fixture:"` and assert production mode
  does NOT return it under a benchmark-shaped query.
- Long-term (Phase 4): add dedicated `fixture_id TEXT` + `gate_brief_hash TEXT`
  columns + index, OR a separate `gate_fixtures` join table. Match by exact column
  equality, not substring.

---

## F5 (M) — F-3.14 PII redaction misses common sensitive headers

**Evidence** (`verifier/extraction.rs:32-34`):

```rust
static AUTH_HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)\b(?:Authorization:\s*Bearer|X-Api-Key:)\s*[^\s]+").expect("valid regex")
});
```

F-3.14's requirement says **"bearer/API-key-like header patterns"** — Architect's
list reasonably extends to anything that carries a secret in a header value. Common
patterns currently NOT caught:

- `Cookie: session=abc...`
- `Set-Cookie: token=...`
- `X-Auth-Token: ...`
- `Proxy-Authorization: Basic ...`
- `X-CSRF-Token: ...`
- `Authentication: ...` (note: variant of `Authorization`)

A leaked session cookie or CSRF token in an extracted playbook is the same severity
class as a leaked bearer token — the redaction floor should match.

**Recommended fix** — extend the alternation in `AUTH_HEADER_RE` to cover the four
most common additions. ~1 line.

---

## F6 (M) — F-3.14 PHONE_RE + IPV4_RE over-redact false positives

**Evidence** (`verifier/extraction.rs:24-29`):

```rust
static PHONE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?x)(?:\+?[1-9]\d{1,14}|\(?\d{2,4}\)?[-.\s]?\d{3,4}[-.\s]?\d{4})")
        .expect("valid regex")
});
static IPV4_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("valid regex"));
```

**Issues**

1. `PHONE_RE` first alternative `\+?[1-9]\d{1,14}` matches *any* sequence of 2-16
   digits starting with non-zero. False-positive matches: order IDs, version numbers
   ("12345"), Unix timestamps, line numbers in stack traces.
2. `IPV4_RE` `\b(?:\d{1,3}\.){3}\d{1,3}\b` matches `1.2.3.4`, `127.0.0.1`, but ALSO
   semantic version strings: `1.2.3.4` as a software version, `2025.10.15.0` as a
   build ID, etc. F-3.14's PII intent is private network identifiers — over-redacting
   harmless versions makes playbook content less useful.

**Why this matters now** — extracted playbooks are injected into Initializer system
prompts (top-3, NFR-3.3 12KB cap). Over-redaction degrades signal density inside
that budget. A playbook that says "deploy version `[REDACTED_IP].[REDACTED_IP]`"
is useless to the agent.

**Recommended fix**

- Tighten `PHONE_RE` to require either `+` prefix OR a separator (`-`, `.`, space)
  somewhere in the match — eliminates the bare-digit false positive.
- Add a heuristic for `IPV4_RE`: skip the match if it's preceded by version-shape
  context like `v`, `version`, `release`, OR if any octet > 255 (which would make
  it not a valid IP — e.g. `2025.10.15.0`).
- Alternative: add an `allowlist_known_versions` flag the extractor sets when
  rendering technical playbooks. Phase 4 Curator can tune.

Acceptable for Phase 3 to ship the current regex with a DEBT entry + a follow-up
test corpus capturing the false-positive cases for Phase 4 tuning.

---

## F7 (M) — Story 3.16 acceptance skipped AGENTS.md §6 gates

**Evidence** — Story 3.16's Verification section runs:

```bash
cargo test phase3_warm_benchmark
cargo test sessions_tool_calls_matches_action_count
cargo test phase3_production_matcher_smoke
cargo test -p seasoned-hand-core fts5::trigger_correctness
bash scripts/spec-check.sh
```

But AGENTS.md §6 mandates:

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
pnpm typecheck
pnpm test
./scripts/spec-check.sh
```

The story's verification ran only the Phase-3-specific tests + spec-check —
**skipping `cargo test --workspace`, `cargo clippy -- -D warnings`, and
`cargo fmt --check`**. These are exactly the gates that would have caught F1/F2/F3.

**Recommended fix** — backfill story 3.16's Verification section to run the full
AGENTS.md §6 gate list. Future close-out stories (Phase 4+) should include the
full gate list by default, not the phase-specific subset.

---

## F8 (L) — Untested orchestrator branch: post-cap quality-floor combined emit

**Evidence** — architecture.md §3 step 6 mandates:

> If the cap fires AND post-cap content drops below the quality floor (rare; only
> triggers when render exceeds 24 KB), emit BOTH `playbook_extraction_output_capped`
> AND `playbook_extraction_rejected{layer:"quality_floor"}` so the audit trail
> captures the actual failure chain.

`verifier/extraction.rs:294-298` has a unit test that simulates the post-cap
floor-fail at the helper level, but no test in the orchestrator (probably
`verifier/gate.rs` extraction-call site) asserts that BOTH events actually get
written to the event stream in the same handler. The combined emit could silently
regress to single-event emit if the orchestrator is later refactored.

**Recommended fix** — add an integration test under `seasoned-hand-core` (in the
extraction orchestrator's test module) that drives a real extractor with an LLM
response that renders >24 KB content with steps that lose their >=200-char floor
after capping, and asserts both Misc events appear in `session_search_index` (or
the events table) for the synthetic session.

---

## Suggested fix order

Apply F1 → F2 → F3 in one commit (cleans the gates).
Apply F5 in same commit (1-line regex extension).
File DEBT seeds for F4, F6, F8 (deeper changes; not Phase 3 blockers).
Backfill F7 by amending story 3.16's verification list (1-line spec edit).

After Claude iter-1 lands these fixes + DEBT, dispatch Codex iter-2 to look for
issues Claude missed.

---

## REVIEW iter-2 (Codex, 2026-05-18)

Scope: independent re-audit after Claude iter-1 (`c6669b2`) with hotspot deep-dive on
V010 migration SQL, `verifier/gate.rs`, session-search serialization, initializer
injection, un-stubbed tools, and cross-phase invariants.

### A) Grading Claude iter-1 findings (F1-F8)

- **Agreed (7/8)**: F1, F2, F3, F5, F6, F7, F8.
  - Severity + root cause are directionally correct.
- **Disagreed (1/8)**: F4 severity.
  - Claude marked **M**; iter-2 downgrades to **L** in current code reality because the
    `fixture:*` / `brief:*` sentinel rows are test harness seeds only, not produced by
    the extraction pipeline. The schema smell remains real (keep DEBT #80), but present
    production impact is limited.

### B) New findings from independent review

#### N1 (H) — Production `VerifierGate` runs without an extraction handler

**Evidence**
- `crates/seasoned-hand-server/src/main.rs:346-349` constructs:
  - `VerifierGate::new(...).with_rollback(...)`
  - but **never** calls `.with_extraction(...)`.
- `VerifierGate::run_sync_extraction()` treats missing handler as:
  - `ExtractionError::new("llm_call", "extraction_handler_not_configured")`
  - and emits `Misc{kind:"playbook_extraction_error", stage:"llm_call", ...}`.
- Repo-wide search shows no production `ExtractionHandler` implementation bound into
  server wiring.

**Impact**
- On every PASS verdict with `tool_calls >= 5`, the runtime emits extraction-error
  telemetry and writes no playbooks. That undermines the Phase 3 learning loop
  contract (F-3.1/F-3.7/F-3.8/F-3.15).

**Disposition**
- Not fixed in iter-2 (requires non-trivial implementation + wiring slice).
- Seeded as **DEBT #84 (H)** below.

#### N2 (M) — `phase3_warm_benchmark` acceptance gate was self-fulfilling

**Evidence (pre-fix)**
- Test set `sessions.tool_calls` directly to `0.70 * cold_baseline`, then asserted
  the same inequality.

**Fix applied in iter-2**
- `crates/seasoned-hand-core/src/verifier/gate.rs` `phase3_warm_benchmark` now:
  - seeds Action events (`warm_action_count`),
  - derives `tool_calls` from Action-event count,
  - asserts `sessions.tool_calls == action_count`,
  - then applies `<= 0.70 * cold_baseline`.
- This removes the tautological write-and-assert pattern and binds the warm gate to
  the same canonical counter wiring used by F-3.6 parity expectations.

### C) Cross-phase regression checks

- **Phase 1 verifier verdict flow**: unchanged transition logic for
  `TaskComplete`/`Invalidation`/`CircuitBreaker` in `verifier/gate.rs`; no new state
  transition regressions found.
- **Phase 2 task lifecycle / task_complete path**: verifier PASS path still emits
  `task_complete` Misc and transitions to `FINISHED`; extraction hook is in-path but
  non-blocking (timeout/error emits + continue).
- **Event-stream append-only invariant**: preserved.
  - `SqliteEventStore::append()` inserts into `events`, then indexes into
    `session_search_index` in the same DB closure/transactional unit.
  - No UPDATE/DELETE introduced on `events`.
- **Tool catalog count**: unchanged.
  - `tools/builtin.rs` still at `39` `map.insert(...)` lines
    (`38` unique + `task_deliver` prod override pattern).

### D) Spec-fidelity trace audit (3 random requirements)

1. **F-3.16 (session search index scope)**
- Req: `specs/phase-3/requirements.md` F-3.16.
- Arch: `specs/phase-3/architecture.md` §3 + per-type searchable_text table.
- Code: `events/sqlite.rs` (`index_event_for_search`) + `events/session_search.rs`
  event-type serializers.
- Tests: `events/tests.rs::session_search::all_event_types_queryable`.
- Status: **implemented; no drift found**.

2. **F-3.11 (top-3 injection, zero-match silent skip)**
- Req: F-3.11.
- Arch: §2.3 `PlaybookInjector`.
- Code: `agent/init/injector.rs` (`take(3)`, truncation behavior) + `agent/init/mod.rs`
  early return on no matches.
- Tests: `injector.rs::tests::top3_behavior`.
- Status: **implemented; no drift found**.

3. **F-3.3 (warm benchmark gate <=0.70x cold baseline)**
- Req: F-3.3.
- Arch: §11 Acceptance gate.
- Code: `verifier/gate.rs::phase3_warm_benchmark`.
- Tests: same test (plus iter-2 hardening above).
- Status: **partially drifted pre-fix (tautological harness); hardened in iter-2**.

### E) Iter-2 fix summary

- Applied M-severity fix: harden `phase3_warm_benchmark` harness to avoid direct
  threshold assignment and bind to Action-event parity.
- Deferred H-severity extraction wiring gap to DEBT #84 with explicit pay-down path.

---

## REVIEW iter-3 (Claude, 2026-05-18)

Independent re-audit after Codex iter-2 (`8a4e19f`). Verified N1 + N2 claims;
extended N1 with the deeper structural finding; added 4 new findings (A3-A6).

### A) Grading Codex iter-2 findings

- **Agreed (2/2)**: N1, N2.
  - **N1 verified**: `grep with_extraction` shows only 3 test sites
    (`gate.rs:971,1044,1068`) + zero production callers. `main.rs:346` constructs
    `VerifierGate::new(...).with_rollback(...)` and stops.
  - **N2 verified**: `phase3_warm_benchmark` rewrite (`gate.rs:1440-1505`) correctly
    seeds N Action events, asserts `sessions.tool_calls == count_action_events()`,
    THEN asserts `<= 0.70 * cold_baseline`. The tautology is closed.
- **Accepted F4 downgrade**: Codex right that gate sentinel coupling is test-only,
  severity L. DEBT #80 stays open for the schema-shape concern.

### B) New findings from iter-3 independent review

#### A1 (H) — Extends N1: NO production `ExtractionHandler` impl exists ANYWHERE

**Evidence**

```
$ grep -rn "impl ExtractionHandler" --include="*.rs"
crates/seasoned-hand-core/src/verifier/gate.rs:949: impl ExtractionHandler for OkExtraction   // #[cfg(test)]
crates/seasoned-hand-core/src/verifier/gate.rs:1022: impl ExtractionHandler for ErrExtraction // #[cfg(test)]
crates/seasoned-hand-core/src/verifier/gate.rs:1051: impl ExtractionHandler for SleepExtraction // #[cfg(test)]
```

main.rs not calling `.with_extraction(...)` is the *visible* gap. The root cause
is **deeper**: there is no production-grade `ExtractionHandler` impl in the
codebase. The architecture's §2.1 LearningExtractor sketch (planner-slot LLM call
→ structured JSON output → glue F-3.13/F-3.14/F-3.18 → write playbook) was never
materialized as a Rust type. The PM persona's story breakdown:

- Story 3.3 shipped the orchestrator scaffolding (`with_extraction` builder,
  timeout wrapper, error-event taxonomy)
- Story 3.4 shipped the helper functions (redaction, adversarial scan,
  quality-floor validator) — but as free functions, not as part of an
  ExtractionHandler implementation
- No story explicitly says "ship the production `ExtractionHandler` that ties
  the planner-slot LLM call to story 3.4's helpers and writes the result"

**Impact** — Phase 3 ships 16/16 stories complete by acceptance-criteria letter,
but the headline learning behavior (extract → match → inject → fewer tool calls)
**does not run end-to-end in production**. Every PASS task with `tool_calls >= 5`
emits `playbook_extraction_error{stage:"llm_call", reason:"extraction_handler_not_configured"}`
to the event stream and writes no playbook.

**Recommended action** — open **story 3.17** (NOT a Phase 4 deferral): ship a
production `PlannerSlotExtractionHandler` that:
1. Resolves `SlotName::Planner` via `SlotRouter`.
2. Builds the extraction prompt with F-3.14 layer-1 abstraction + F-3.13 layer-1
   refusal guidance baked in.
3. Calls the LLM, parses structured JSON output `{title, trigger_keywords,
   overview, steps}`.
4. Applies F-3.14 layer-2 PII redaction + F-3.13 layer-2 adversarial scan +
   F-3.18 quality-floor validator (these helpers already exist).
5. Renders to `content`, applies NFR-3.5 byte cap, writes to `playbooks` table.
6. Wires it in `seasoned-hand-server/src/main.rs:346` via `.with_extraction(...)`.
7. Adds a real end-to-end test driving a stub LLM through the full extraction →
   match → inject → counter-update loop.

This MUST close before Phase 4 starts — otherwise Phase 4 Curator has nothing
to curate.

#### A2 (M) — `phase3_warm_benchmark` still scenario-driven, not loop-driven

**Evidence** — `verifier/gate.rs:1440-1505` (post iter-2):

- Seeds Action events: `for seq in 0..warm_action_count { store.append(Action) }`
- Sets `sessions.tool_calls` to that count
- Asserts threshold

The test never actually drives a warm task through the agent loop. It asserts
"if I synthesize a session that LOOKS like a fast warm run, the threshold check
passes". Iter-2's parity fix closed the worst tautology (the direct threshold
write), but the test still proves the GATE works correctly, not that LEARNING
actually reduces tool calls.

**Disposition** — Downstream of A1. Once story 3.17 ships the production handler,
the warm benchmark can drive a real cold→warm loop with a stub LLM. Until then,
the current test is the best proxy. Severity M (was H if not for the gate fix);
DEBT-track for the same close-out as A1.

#### A3 (L) — Status docs not updated for Phase 3 close-out

**Evidence**

- `/BASELINE.md` line 18: `Status: Phase 2 complete → Phase 3 starting`
- `/AGENTS.md` §13: `Phase: 2 complete → Phase 3 starting`
- Phase 3 is `Status: done` per requirements.md §4 + all 16 story files.

**Recommended fix** — flip both to `Phase 3 complete → Phase 4 starting` (after
A1 is closed; otherwise honestly: `Phase 3 partial — extraction handler pending`).

#### A4 (L) — CHANGELOG.md missing `[0.3.0]` entry for Phase 3

**Evidence** — `CHANGELOG.md` has `[0.2.0] — 2026-05-16` Phase 2 release section,
nothing for Phase 3. Phase 2 set the precedent that each phase ships with a
CHANGELOG version bump.

**Recommended fix** — add `[0.3.0] — 2026-05-18` section after Phase 3 close-out
(post-A1). Should include the V010 + ADR-012 + ARCH v1.2 highlights, the 16-story
breakdown, and the known A1 caveat if closing before Phase 4 isn't feasible.

#### A5 (M) — `events/session_search.rs` `collect_json_strings` indexes JSON KEYS

**Evidence** (`events/session_search.rs:291-309`):

```rust
fn collect_json_strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        ...
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                out.push(k.clone());   // <-- KEY pushed into searchable_text
                collect_json_strings(v, out);
            }
        }
    }
}
```

`searchable_text_for_event` then calls `flatten_json_values(&event.data)` which
pulls every object key into the FTS index. So `playbooks_fts` / `session_search_fts`
both tokenize field names: `kind`, `playbook_id`, `tool_name`, `stage`, `reason`,
`matcher_mode`, `original_bytes`, etc.

**Impact** — operator searching for `kind` or `playbook_id` will match every Skill
event and most Misc events, regardless of relevance. Per-EventType shape table
in architecture §3 describes indexing VALUES (kind value, tool_name value, ...);
keys are metadata that operators rarely search by.

**Recommended fix** — drop the `out.push(k.clone())` line. Object-key indexing is
not part of any documented shape. 1-line change.

#### A6 (L) — Searchable-text double-counts explicit + flatten fields

**Evidence** — every match arm in `searchable_text_for_event` extracts specific
fields (e.g. `field_string(data, &["kind"])`) then appends
`flatten_json_values(&event.data)` which includes the same fields again.

**Impact** — duplicates appear in `searchable_text`. FTS5 weighting is mildly
skewed (search for `match` hits twice in any `Skill{kind:"match"}` row). Storage
overhead is small.

**Recommended fix** — drop the trailing `flatten_json_values` from the Message /
Plan / Skill / Misc arms; rely on the explicit field extraction. Defer to A5's
fix slice or to a small follow-up.

### C) Iter-3 fix summary

- **A5 fixed inline**: removed JSON-key push from `collect_json_strings`.
- **A1 + A2 deferred** to **story 3.17** (NOT a Phase 4 work item — must close
  before Phase 4 starts). DEBT #84 already covers A1 from Codex iter-2;
  promote severity to "Phase 4 BLOCKER" and add iter-3 evidence.
- **A3 + A4** deferred: should land paired with story 3.17 close-out (status
  flip + CHANGELOG entry are honest signal only when the learning loop actually
  loops).
- **A6** DEBT-tracked for editorial cleanup.

### D) Recommendation

**Phase 3 is functionally incomplete.** All 16 stories pass acceptance criteria
at the letter, but the headline learning behavior does not run in production
(A1). The Phase 3 "complete" claim should not stand until story 3.17 ships.

Two options:
1. **Open story 3.17 now** and ship the production `ExtractionHandler` before
   any Phase 4 work begins. ~3-5h of work; mirrors the architecture §2.1
   LearningExtractor sketch.
2. **Mark Phase 3 as partial-complete** in BASELINE.md / AGENTS.md / CHANGELOG.md
   with an explicit caveat, defer A1 to Phase 4-day-1, and proceed to Phase 4
   architecture pass knowing the extraction handler is the first Phase 4 story.

Both options need user direction. Recommend option 1 — Phase 4 Curator has
nothing to curate without A1 closed.

---

## REVIEW iter-4 (Claude, 2026-05-18)

Scope: review of Codex's story 3.17 implementation (`e4daca1`) — 487-line
`extraction_handler.rs` + main.rs wiring + 3 integration tests + warm benchmark
update. Per "iterate until no issue found" discipline.

### Findings

#### C1 (M) — Transcript window reads FIRST 200 events, not LAST 200 (FIXED inline)

**Evidence** (`extraction_handler.rs:81-87`):

```rust
let mut stmt = conn.prepare(
    "SELECT type, source, data
     FROM events
     WHERE session_id = ?
     ORDER BY id ASC
     LIMIT 200",
)?;
```

A Phase 3 task with 50+ tool calls produces 100+ events (Action + Observation
per call, plus Plan + Misc). LIMIT 200 ORDER BY id ASC keeps the FIRST 200 —
typically session setup / plan creation — and DROPS the mid-task procedure body
where the actual reusable workflow happens. Extraction quality silently degrades
on long tasks.

**Fix applied** — switched to `ORDER BY id DESC LIMIT 200` + reverse in memory,
so the LLM sees the most-recent 200 events in time order. The tail of the task
(where the procedure converges + verifier passes) is the relevant slice.

#### C2 (M, security) — F-3.14 redaction skips title + trigger_keywords (FIXED inline)

**Evidence** (`extraction_handler.rs:172-195`, pre-fix) — `redact_pii` was only
called on `parsed.overview` and each `parsed.steps[i]`. The `title` field and
the `trigger_keywords[]` strings went straight to the INSERT without redaction.

**Impact** — an LLM that leaks an email / bearer token / phone number into the
playbook title or trigger keywords bypasses F-3.14 layer-2 redaction. The title
is operator-visible in `seasoned-hand playbook list/show` output; trigger keywords
are FTS5-indexed and matchable. Both are PII surface area equal to overview/steps.

**Fix applied** — generalized redaction to ALL LLM-produced text fields:
`title`, `overview`, every entry of `steps[]`, every entry of `trigger_keywords[]`.
PII counts and categories accumulate across all fields into the single
`playbook_extraction_pii_redacted` event.

#### C3 (M) — `phase3_warm_benchmark` exercises matcher+injector but NOT extraction

**Evidence** (`verifier/gate.rs:1442-1505`) — warm benchmark now:
1. Seeds cold session w/ Action events
2. Hand-seeds a fixture playbook via `seed_gate_fixture_playbook`
3. Calls `match_playbooks(MatcherMode::Gate)` + `build_injection`
4. Asserts matcher hit + injection non-empty
5. Seeds warm Action events at 70%
6. Asserts threshold

Improvement over iter-2 (now actually exercises matcher + injector), but the
playbook is HAND-SEEDED, not produced by the extraction handler. The end-to-end
loop (extract → match → inject → counter) is split across `end_to_end_loop`
test (extract path) and `phase3_warm_benchmark` (match+inject path) but never
tested in one flow.

**Disposition** — DEBT-tracked (#85 is partial-close). Closing fully requires
a benchmark that runs extraction → match → inject as one transaction.

#### C4 (L) — New event kind `playbook_extraction_written` undocumented

**Evidence** (`extraction_handler.rs:293-298`) — handler emits
`Misc{kind:"playbook_extraction_written", playbook_id}` on success path. This
kind isn't enumerated in architecture §4 alongside the other six
`playbook_extraction_*` kinds.

**Disposition** — DEBT-tracked editorial. Architecture §4 should be updated
to acknowledge the success-path emit (it's useful operator telemetry — count
of playbooks ACTUALLY written vs reasons for skipping).

#### C5 (L) — LLM refusal-guidance system prompt is vague

**Evidence** (`extraction_handler.rs:122`):

```
"...Do not draft playbooks that include shell substitutions, raw external IP
URLs, role-reversal markers, prompt-injection patterns, or opaque blobs."
```

Doesn't enumerate the specific phrases the deterministic layer (F-3.13 layer 2)
will reject. The LLM's layer-1 protection is weaker than it could be — the
deterministic layer catches the gap, but a tighter prompt reduces redundant
post-hoc rejection.

**Disposition** — DEBT-tracked editorial. Phase 4 may tune from rejection
telemetry.

#### C6 (L) — No dedup guard for re-triggered extraction

**Evidence** — `extract_sync` always issues `INSERT INTO playbooks` with a
fresh `pb-{uuid}`. If extraction fires twice for the same session (e.g. retry
after transient gate-side failure), two playbook rows are created with same
`source_task_id`. Both survive, both match future tasks. F-3.7 implies
once-per-task.

**Disposition** — DEBT-tracked. Add a guard
`SELECT 1 FROM playbooks WHERE source_task_id = ? LIMIT 1` before insert; if
extant, emit `playbook_extraction_skipped{reason:"duplicate"}` and return.

### Iter-4 fix summary

- **C1 + C2 FIXED inline**: transcript reads last 200 events; PII redaction
  covers all 4 LLM-produced fields.
- **C3 / C4 / C5 / C6** seeded as DEBT entries for Phase 4 follow-up. None
  are Phase 4 BLOCKERS — the learning loop now genuinely closes in production
  thanks to story 3.17; these are refinements.

### Recommendation

Phase 3 is functionally complete after story 3.17 + iter-4 C1/C2 fixes.
- Headline learning loop closes end-to-end (extract → match → inject → counter)
- All 6 AGENTS.md §6 gates green
- Security floor (F-3.13/F-3.14) applies to all LLM-emitted fields
- 17/17 stories Status: done

If the user wants iter-5 (Codex review of iter-4) for symmetry, dispatch
Codex; otherwise call hardening complete and proceed to Phase 4 architecture
pass.
