#!/usr/bin/env bash
# Seasoned Hand — Repository Setup Script
#
# What this does:
#   1. Verifies you're in the right directory
#   2. Initializes git repo
#   3. Makes initial commit
#   4. Optionally adds GitHub remote and pushes
#
# What this does NOT do:
#   - Create a GitHub repo (you do this on github.com first)
#   - Install dependencies (Rust, Node, etc.)
#
# Usage:
#   cd seasoned-hand
#   bash setup-repo.sh

set -euo pipefail

GH_USER="slowdoctor-dev"
REPO_NAME="seasoned-hand"

# ─── Colors ────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# ─── Header ────────────────────────────────────────────────────
echo ""
echo -e "${BLUE}╔════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║   Seasoned Hand — Repository Setup         ║${NC}"
echo -e "${BLUE}║   Every task makes the hand wiser.         ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════╝${NC}"
echo ""
echo "  Owner: $GH_USER"
echo "  Repo:  $REPO_NAME"
echo ""

# ─── Verify we're in the right directory ───────────────────────
if [ ! -f "BASELINE.md" ] || [ ! -f "AGENTS.md" ]; then
  echo -e "${RED}ERROR: Run this from the seasoned-hand directory.${NC}"
  echo "Current directory: $(pwd)"
  echo "Expected files here: BASELINE.md, AGENTS.md, README.md, ..."
  exit 1
fi

echo -e "${GREEN}✓${NC} Running from project root"

# ─── Verify prerequisites ──────────────────────────────────────
if ! command -v git &> /dev/null; then
  echo -e "${RED}ERROR: git is not installed.${NC}"
  exit 1
fi
echo -e "${GREEN}✓${NC} git: $(git --version | head -1)"

# ─── Check git user config ─────────────────────────────────────
if ! git config user.name &> /dev/null; then
  echo -e "${YELLOW}!${NC} git user.name not set globally. Set with:"
  echo "  git config --global user.name 'Your Name'"
  echo "  git config --global user.email 'your-email@example.com'"
  read -p "Continue anyway? (y/N) " -n 1 -r
  echo
  [[ ! $REPLY =~ ^[Yy]$ ]] && exit 1
else
  echo -e "${GREEN}✓${NC} git user: $(git config user.name) <$(git config user.email)>"
fi

# ─── Check if already a git repo ───────────────────────────────
if [ -d ".git" ]; then
  echo ""
  echo -e "${YELLOW}WARNING: This is already a git repository.${NC}"
  read -p "Continue anyway? (y/N) " -n 1 -r
  echo
  [[ ! $REPLY =~ ^[Yy]$ ]] && { echo "Aborted."; exit 1; }
else
  # ─── Initialize git ─────────────────────────────────────────
  echo ""
  echo "Initializing git repository..."
  git init -q
  git branch -M main
  echo -e "${GREEN}✓${NC} git init + main branch"
fi

# ─── Final safety check ────────────────────────────────────────
echo ""
echo "Final safety check..."
REMAINING=$(grep -rln "<your-username>\|<your-handle>\|<owner>" . 2>/dev/null | grep -v "setup-repo.sh" || true)
if [ -n "$REMAINING" ]; then
  echo -e "${YELLOW}WARNING: Unfilled placeholders found:${NC}"
  echo "$REMAINING"
  read -p "Continue anyway? (y/N) " -n 1 -r
  echo
  [[ ! $REPLY =~ ^[Yy]$ ]] && exit 1
else
  echo -e "${GREEN}✓${NC} No unfilled placeholders"
fi

# Check for accidentally included .env
if [ -f ".env" ]; then
  echo -e "${RED}ERROR: .env file found. Remove before committing.${NC}"
  echo "  rm .env"
  exit 1
fi
echo -e "${GREEN}✓${NC} No .env file (good)"

# ─── First commit ──────────────────────────────────────────────
echo ""
read -p "Make initial commit now? (Y/n) " -n 1 -r
echo

if [[ ! $REPLY =~ ^[Nn]$ ]]; then
  git add .
  # Exclude this script from the commit (one-time use)
  git reset HEAD setup-repo.sh 2>/dev/null || true
  
  git commit -q -m "chore: initial scaffold

- BASELINE.md as single entry point
- AGENTS.md as LLM-agnostic source of truth
- 10 ADRs documenting key architectural decisions
- 17 principles (incl. RAM/disk dichotomy and plan-as-PCB)
- Phase 0 requirements (27 stories, foundation skeleton)
- 6-phase roadmap (22 weeks total)
- BMAD/GSD methodology integrated
- External validation via Manus direct Q&A (specs/07-research/)

Stack: Bifrost gateway + Rust control plane + Next.js frontend
Phase: -1 (planning complete) → Phase 0 starting

Every task makes the hand wiser."
  
  echo -e "${GREEN}✓${NC} Initial commit created"
  echo ""
  git log --oneline -1
else
  echo "Skipped initial commit. Run manually:"
  echo "  git add . && git reset HEAD setup-repo.sh"
  echo "  git commit -m 'chore: initial scaffold'"
fi

# ─── Optional: add remote and push ─────────────────────────────
echo ""
echo "GitHub repository should be at:"
echo -e "  ${BLUE}https://github.com/$GH_USER/$REPO_NAME${NC}"
echo ""
echo "(Recommended: create as Private, switch to Public after Phase 0)"
echo ""
read -p "Add remote and push now? (y/N) " -n 1 -r
echo

if [[ $REPLY =~ ^[Yy]$ ]]; then
  if git remote get-url origin &> /dev/null; then
    echo -e "${YELLOW}!${NC} Remote 'origin' already exists:"
    git remote get-url origin
    read -p "Replace? (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
      git remote set-url origin "git@github.com:$GH_USER/$REPO_NAME.git"
    fi
  else
    git remote add origin "git@github.com:$GH_USER/$REPO_NAME.git"
  fi
  echo -e "${GREEN}✓${NC} Remote: git@github.com:$GH_USER/$REPO_NAME.git"
  
  echo ""
  echo "Pushing to main..."
  if git push -u origin main; then
    echo -e "${GREEN}✓${NC} Pushed to GitHub"
    echo ""
    echo -e "View at: ${BLUE}https://github.com/$GH_USER/$REPO_NAME${NC}"
  else
    echo ""
    echo -e "${RED}!${NC} Push failed. Common causes:"
    echo "  - GitHub repo not created yet (go to github.com/new)"
    echo "  - SSH key not configured (test: ssh -T git@github.com)"
    echo "  - Wrong repo name (expected: $GH_USER/$REPO_NAME)"
    echo ""
    echo "Try HTTPS instead:"
    echo "  git remote set-url origin https://github.com/$GH_USER/$REPO_NAME.git"
    echo "  git push -u origin main"
  fi
else
  echo ""
  echo "Skipped. When GitHub repo is ready:"
  echo -e "  ${BLUE}git remote add origin git@github.com:$GH_USER/$REPO_NAME.git${NC}"
  echo -e "  ${BLUE}git push -u origin main${NC}"
fi

# ─── Final guidance ────────────────────────────────────────────
echo ""
echo -e "${GREEN}╔════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║  Setup complete                            ║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════════╝${NC}"
echo ""
echo "Next steps:"
echo "  1. docs/setup-checklist.md — domain/account setup"
echo "  2. docs/first-week-plan.md — Day 1-7 actions"
echo "  3. Day 2: BMAD Architect persona → Phase 0 architecture"
echo "       Run: just architect-prompt"
echo "  4. Day 3: BMAD PM persona → break 26 stories"
echo "       Run: just pm-prompt"
echo "  5. Day 4: Implement Story 0.1 (Bifrost Docker setup)"
echo "       Run: just story-prompt"
echo ""
echo "When starting any fresh AI session:"
echo -e "  ${BLUE}Read BASELINE.md first.${NC}"
echo ""
echo "Every task makes the hand wiser."
echo ""

# ─── Self-cleanup ──────────────────────────────────────────────
read -p "Remove this setup script? (Y/n) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Nn]$ ]]; then
  rm -- "$0"
  echo "setup-repo.sh removed."
fi
