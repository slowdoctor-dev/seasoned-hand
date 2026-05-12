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

# Check 5: Tool catalog registry size (Phase 1 story 1.4)
# Phase 1 baseline now ships 35 (Phase 0's 33 + feature_mark_done + progress_update).
TOOLS_BUILTIN=crates/seasoned-hand-core/src/tools/builtin.rs
if [ -f "$TOOLS_BUILTIN" ]; then
  count=$(grep -c "map.insert(" "$TOOLS_BUILTIN" || echo 0)
  expected=35
  if [ "$count" -ne "$expected" ]; then
    echo "✗ Tool catalog registry has $count entries, expected $expected (story 0.7 / DEBT #4)"
    FAIL=$((FAIL+1))
  else
    echo "✓ Tool catalog registry size ($count) matches story 0.7"
    PASS=$((PASS+1))
  fi
fi

# Check 6: Required top-level files exist
for f in CLAUDE.md AGENTS.md README.md specs/01-architecture/ARCHITECTURE.md docs/methodology.md; do
  check "$f exists" "[ -f '$f' ]"
done

# Check 7: Story 1.5 tool catalog stability test guard
check "tool_catalog_order_is_stable test exists" \
  "grep -q 'fn tool_catalog_order_is_stable' crates/seasoned-hand-core/src/dispatch/mask.rs"

echo ""
echo "=== Results ==="
echo "Pass: $PASS"
echo "Fail: $FAIL"

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
