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
