#!/bin/bash
# scripts/status.sh
# Shows current phase, active story, blockers.

set -e

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "=== Seasoned Hand — Status ==="
echo ""

# Current phase: find latest phase with stories
LATEST_PHASE=""
for p in specs/phase-*/; do
  LATEST_PHASE="$p"
done

if [ -z "$LATEST_PHASE" ]; then
  echo "No phases yet. Run 'just analyst-prompt' to start Phase 0."
  exit 0
fi

PHASE_NUM=$(basename "$LATEST_PHASE" | sed 's/phase-//')
echo "Current phase: $PHASE_NUM"
echo ""

# Story stats
if [ -d "${LATEST_PHASE}stories" ]; then
  TOTAL=$(ls -1 "${LATEST_PHASE}stories"/story-*.md 2>/dev/null | wc -l)
  DONE=$(grep -l "^> \*\*Status\*\*: done" "${LATEST_PHASE}stories"/story-*.md 2>/dev/null | wc -l)
  IN_PROGRESS=$(grep -l "^> \*\*Status\*\*: in-progress" "${LATEST_PHASE}stories"/story-*.md 2>/dev/null | wc -l)
  BLOCKED=$(grep -l "^> \*\*Status\*\*: blocked" "${LATEST_PHASE}stories"/story-*.md 2>/dev/null | wc -l)
  READY=$((TOTAL - DONE - IN_PROGRESS - BLOCKED))
  
  echo "Stories: $TOTAL total"
  echo "  Done:        $DONE"
  echo "  In-progress: $IN_PROGRESS"
  echo "  Ready:       $READY"
  echo "  Blocked:     $BLOCKED"
  echo ""
  
  # In-progress stories
  if [ "$IN_PROGRESS" -gt 0 ]; then
    echo "Active stories:"
    grep -l "^> \*\*Status\*\*: in-progress" "${LATEST_PHASE}stories"/story-*.md 2>/dev/null | while read f; do
      title=$(grep -m1 "^# Story " "$f" | sed 's/^# Story [0-9.]* — //')
      echo "  - $(basename "$f" .md): $title"
    done
    echo ""
  fi
  
  # Blocked
  if [ "$BLOCKED" -gt 0 ]; then
    echo "⚠ Blocked stories:"
    grep -l "^> \*\*Status\*\*: blocked" "${LATEST_PHASE}stories"/story-*.md 2>/dev/null | while read f; do
      title=$(grep -m1 "^# Story " "$f" | sed 's/^# Story [0-9.]* — //')
      echo "  - $(basename "$f" .md): $title"
    done
    echo ""
  fi
  
  # Next ready
  if [ "$READY" -gt 0 ]; then
    NEXT=$(grep -L -E "^> \*\*Status\*\*: (done|in-progress|blocked)" "${LATEST_PHASE}stories"/story-*.md 2>/dev/null | head -1)
    if [ -n "$NEXT" ]; then
      title=$(grep -m1 "^# Story " "$NEXT" | sed 's/^# Story [0-9.]* — //')
      echo "Next ready: $(basename "$NEXT" .md): $title"
    fi
  fi
fi

echo ""

# Git status summary
if [ -d ".git" ]; then
  UNTRACKED=$(git status --porcelain 2>/dev/null | wc -l)
  if [ "$UNTRACKED" -gt 0 ]; then
    echo "⚠ Uncommitted changes: $UNTRACKED files"
  fi
  BRANCH=$(git branch --show-current 2>/dev/null)
  echo "Branch: $BRANCH"
fi
