#!/bin/bash
# scripts/spec-check.sh
# Verifies code matches /specs.
# Runs as part of `just verify` gate.

set -e

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PASS=0
FAIL=0

check() {
  local name="$1"
  local cmd="$2"
  if eval "$cmd" > /dev/null 2>&1; then
    echo "✓ $name"
    PASS=$((PASS+1))
  else
    echo "✗ $name"
    FAIL=$((FAIL+1))
  fi
}

echo "=== Spec compliance check ==="

# Check 1: All Phase N requirements have corresponding architecture
for req in specs/phase-*/requirements.md; do
  phase_dir=$(dirname "$req")
  arch="$phase_dir/architecture.md"
  if [ -f "$req" ] && [ ! -f "$arch" ]; then
    echo "✗ Missing architecture.md for $(basename "$phase_dir")"
    FAIL=$((FAIL+1))
  fi
done

# Check 2: All stories referenced in requirements exist
# (skipping for v0 — too brittle without parser)

# Check 3: All Rust modules have spec references
# Each Rust module top should have // refs: /specs/...
if [ -d "src" ]; then
  missing=$(find src -name "*.rs" -exec grep -L "refs: /specs/" {} \; | wc -l)
  if [ "$missing" -gt 0 ]; then
    echo "⚠ $missing Rust files missing /specs/ ref comment"
  fi
fi

# Check 4: No orphan stories (status: done but file changed since)
# (skipping for v0)

# Check 5: Tool catalog registry size (Phase 1 stories 1.4 + 1.13 + 1.13b;
# Phase 2 story 2.14 adds task_deliver).
# Phase 2 baseline ships 38: Phase 0's 33 + feature_mark_done +
# progress_update (story 1.4) + checkpoint_label (story 1.13) +
# checkpoint_rollback (story 1.13b — registered but masked from LLM)
# + task_deliver (story 2.14 — Worker-mode only).
TOOLS_BUILTIN=crates/seasoned-hand-core/src/tools/builtin.rs
if [ -f "$TOOLS_BUILTIN" ]; then
  # Raw `map.insert(` lines = 39 since story 2.14 (one insert in `all()`
  # for the not-wired placeholder, one in `all_with_task_deliver()` for
  # the production override). Unique tool count is 38.
  count=$(grep -c "map.insert(" "$TOOLS_BUILTIN" || echo 0)
  expected=39
  if [ "$count" -ne "$expected" ]; then
    echo "✗ Tool catalog registry has $count entries, expected $expected (story 0.7 / 2.14; 38 unique + task_deliver prod override)"
    FAIL=$((FAIL+1))
  else
    echo "✓ Tool catalog registry size ($count = 38 unique + task_deliver prod override) matches story 2.14"
    PASS=$((PASS+1))
  fi
fi

# Check 6: Required top-level files exist
for f in CLAUDE.md AGENTS.md README.md specs/01-architecture/ARCHITECTURE.md docs/methodology.md; do
  check "$f exists" "[ -f '$f' ]"
done

# Check 7: Story 1.5 guard + Phase 3 hook (story 3.16, closes Phase 2 DEBT #62)
check "tool catalog stability + Phase 3 spec hook" \
  "grep -q 'fn tool_catalog_order_is_stable' crates/seasoned-hand-core/src/dispatch/mask.rs \
   && [ -f migrations/V010__phase3_learning_artifacts.sql ] \
   && grep -q 'CREATE TABLE sops' migrations/V010__phase3_learning_artifacts.sql \
   && grep -q 'CREATE TABLE glossary' migrations/V010__phase3_learning_artifacts.sql \
   && grep -Eq 'v1\\.(2|3)' specs/01-architecture/ARCHITECTURE.md \
   && grep -q 'sop create / edit / list / show / delete' specs/phase-3/architecture.md \
   && grep -q 'playbook list / show / delete' specs/phase-3/architecture.md \
   && grep -q 'session search <query>' specs/phase-3/architecture.md"

# Check 8: Phase 4 close-out hook (story 4.22 — Curator schema V011/V012,
# retention module, architecture v1.3 reconciliation).
check "Phase 4 curator + retention spec hook" \
  "[ -f migrations/V011__phase4_curator.sql ] \
   && [ -f migrations/V012__phase4_curator_retention.sql ] \
   && grep -q 'CREATE TABLE curator_decisions' migrations/V011__phase4_curator.sql \
   && grep -q 'CREATE TABLE curator_review_queue' migrations/V011__phase4_curator.sql \
   && grep -q 'CREATE VIRTUAL TABLE curator_search_fts' migrations/V011__phase4_curator.sql \
   && grep -q 'CREATE TABLE curator_decisions_summary' migrations/V012__phase4_curator_retention.sql \
   && [ -f crates/seasoned-hand-core/src/curator/mod.rs ] \
   && [ -f crates/seasoned-hand-core/src/curator/retention.rs ] \
   && grep -q 'pub struct CuratorRetentionJob' crates/seasoned-hand-core/src/curator/retention.rs \
   && grep -Eq 'v1\\.3' specs/01-architecture/ARCHITECTURE.md \
   && [ -f specs/phase-4/architecture.md ] \
   && [ -f specs/phase-4/requirements.md ]"

# Check 9: Phase 5 per-crate dependency justification (story 5.23 /
# closes DEBT #97). Asserts the ARCHITECTURE.md §1 addendum block
# carries an explicit Phase 5 entry. Future stories that add a
# workspace dependency must extend the block; this gate prevents
# silent dep additions from accumulating.
check "Phase 5 dependency addendum present" \
  "grep -q 'Phase 5 dependency addendum' specs/01-architecture/ARCHITECTURE.md"

# Check 10: Phase 5 close-out hook (story 5.33). Pins the load-bearing
# Phase 5 schema + module surface so a future refactor can't silently
# unwind the multi-user + RBAC + audit layer.
check "Phase 5 close-out spec hook" \
  "[ -f migrations/V013__phase5_tenant_rbac_audit.sql ] \
   && grep -q 'CREATE TABLE IF NOT EXISTS organizations' migrations/V013__phase5_tenant_rbac_audit.sql \
   && grep -q 'CREATE TABLE IF NOT EXISTS users' migrations/V013__phase5_tenant_rbac_audit.sql \
   && grep -q 'CREATE TABLE IF NOT EXISTS organization_memberships' migrations/V013__phase5_tenant_rbac_audit.sql \
   && grep -q 'CREATE TABLE IF NOT EXISTS audit_log' migrations/V013__phase5_tenant_rbac_audit.sql \
   && grep -q 'CREATE TABLE IF NOT EXISTS user_cost_ledger' migrations/V013__phase5_tenant_rbac_audit.sql \
   && grep -q 'CREATE TABLE IF NOT EXISTS tenant_event_view' migrations/V013__phase5_tenant_rbac_audit.sql \
   && [ -f crates/seasoned-hand-core/src/auth/policy.rs ] \
   && [ -f crates/seasoned-hand-core/src/audit/ledger.rs ] \
   && [ -f crates/seasoned-hand-core/src/events/visibility.rs ] \
   && [ -f crates/seasoned-hand-core/src/billing/user_cost.rs ] \
   && [ -f crates/seasoned-hand-core/src/handoff/task.rs ] \
   && [ -f crates/seasoned-hand-core/src/org/deactivation.rs ] \
   && [ -f crates/seasoned-hand-core/src/config/strict.rs ] \
   && grep -Eq 'v1\\.4' specs/01-architecture/ARCHITECTURE.md \
   && [ -f specs/phase-5/architecture.md ] \
   && [ -f specs/phase-5/requirements.md ]"

# Check 11: Issue #9 / V021 task hierarchy regression guard.
# V014 rebuilt `tasks` and accidentally dropped the V006 self-FK on
# `parent_task_id` plus the parent/schedule indexes. Pin the restoring
# migration and also reject any later tasks table rebuild that omits the
# same contract.
check "Task parent FK + schedule index regression guard" \
  "[ -f migrations/V021__restore_task_parent_fk_and_indexes.sql ] \
   && grep -q 'parent_task_id[[:space:]]*TEXT REFERENCES tasks(id)' migrations/V021__restore_task_parent_fk_and_indexes.sql \
   && grep -q 'CREATE INDEX idx_tasks_parent[[:space:]]*ON tasks(parent_task_id)' migrations/V021__restore_task_parent_fk_and_indexes.sql \
   && grep -q 'CREATE INDEX idx_tasks_schedule[[:space:]]*ON tasks(schedule) WHERE schedule IS NOT NULL' migrations/V021__restore_task_parent_fk_and_indexes.sql \
   && ! find migrations -name 'V0[2-9][2-9]__*.sql' -print0 | xargs -0 grep -L 'parent_task_id[[:space:]]*TEXT REFERENCES tasks(id)' | xargs grep -l 'CREATE TABLE tasks_' \
   && ! find migrations -name 'V0[2-9][2-9]__*.sql' -print0 | xargs -0 grep -L 'CREATE INDEX idx_tasks_parent[[:space:]]*ON tasks(parent_task_id)' | xargs grep -l 'CREATE TABLE tasks_' \
   && ! find migrations -name 'V0[2-9][2-9]__*.sql' -print0 | xargs -0 grep -L 'CREATE INDEX idx_tasks_schedule[[:space:]]*ON tasks(schedule) WHERE schedule IS NOT NULL' | xargs grep -l 'CREATE TABLE tasks_'"

# Check 12: Issue #15 / V023 session-search FTS tokenizer guard.
# V018 rebuilt `session_search_fts` for RBAC visibility columns but dropped
# V010's unicode61 diacritic folding tokenizer. Pin the forward migration so
# future FTS rebuilds do not silently regress café -> cafe matching again.
check "Session search FTS diacritic folding tokenizer guard" \
  "[ -f migrations/V023__restore_session_search_fts_tokenizer.sql ] \
   && grep -q \"CREATE VIRTUAL TABLE session_search_fts USING fts5\" migrations/V023__restore_session_search_fts_tokenizer.sql \
   && grep -q \"tenant_id UNINDEXED\" migrations/V023__restore_session_search_fts_tokenizer.sql \
   && grep -q \"visibility_level UNINDEXED\" migrations/V023__restore_session_search_fts_tokenizer.sql \
   && grep -q \"tokenize='unicode61 remove_diacritics 2'\" migrations/V023__restore_session_search_fts_tokenizer.sql \
   && ! find migrations -name 'V0[2-9][4-9]__*.sql' -print0 | xargs -0 grep -L \"tokenize='unicode61 remove_diacritics 2'\" | xargs grep -l 'CREATE VIRTUAL TABLE session_search_fts'"

# Check 13: Issue #16 / V024 SOP tenant-scope guard.
# SOP shares are tenant-scoped; the SOP row must be tenant-scoped too so
# an admin cannot share a foreign tenant's SOP id into their own tenant.
check "SOP tenant_id regression guard" \
  "[ -f migrations/V024__tenant_scope_sops.sql ] \
   && grep -q \"ALTER TABLE sops ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'legacy-default'\" migrations/V024__tenant_scope_sops.sql \
   && grep -q \"CREATE INDEX IF NOT EXISTS idx_sops_tenant ON sops(tenant_id)\" migrations/V024__tenant_scope_sops.sql \
   && grep -q \"SELECT 1 FROM sops WHERE id = ? AND tenant_id = ?\" crates/seasoned-hand-core/src/sharing/sop.rs"

echo ""
echo "=== Results ==="
echo "Pass: $PASS"
echo "Fail: $FAIL"

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
