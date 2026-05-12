# GSD Story Execution Prompt

> Use for daily story implementation.
> Works with any AI coding agent (Claude Code, Codex CLI, Cursor, Cline, Aider).
> Start a fresh session and paste this prompt.

Follow the 4-phase GSD workflow: Discuss → Plan → Execute → Verify.

---

You are implementing **one story** for Seasoned Hand.

## Step 0: Load context (mandatory)

Read in order:
1. `/AGENTS.md` (source of truth)
2. `/specs/01-architecture/ARCHITECTURE.md`
3. The story file: `/specs/phase-N/stories/story-N.X.md`
4. Files referenced by the story

Confirm out loud:
> "I've read [files]. The story is to [goal]. The acceptance criteria are [list]."

## Step 1: Discuss (5 min)

Surface uncertainties before planning:
- Anything in the story unclear?
- Anything that conflicts with `/specs/01-architecture/ARCHITECTURE.md`?
- Any technical decisions not in the spec?

If anything is unclear, ask. **Don't proceed with assumptions**.

## Step 2: Plan (15 min)

Output a structured plan:

```
## Plan for Story N.X

### Files to create
- path/to/new/file.rs - purpose
- ...

### Files to modify
- path/to/existing.rs - what changes
- ...

### Tests to add
- test name - what it verifies
- ...

### Verification steps
1. ...
2. ...

### Estimated time
N minutes
```

**Wait for me to say "go" before writing any code.**

## Step 3: Execute

Implement the plan. Rules:
- Write tests first when possible (TDD-style)
- One file at a time, smallest change that works
- After each meaningful change, run the relevant test
- If verification fails, fix or roll back — don't add hacks

## Step 4: Verify

Before declaring done, run:

```bash
# Backend (if changed)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace

# Frontend (if changed)
pnpm typecheck
pnpm test

# Spec compliance
./scripts/spec-check.sh

# Story-specific verification (from story file)
[run commands from story's Verification section]
```

All must pass. If anything fails:
1. Show the failure
2. Diagnose
3. Fix
4. Re-run

## Step 5: Commit

Use exact commit message from story file. One commit per story.

```bash
git add .
git commit -m "<message from story>"
```

## Step 6: Update story status

In the story file, change:
- `Status: ready` → `Status: done`
- Add a brief "Notes from execution" section if anything noteworthy happened

---

## Rules

- ❌ Don't carry state to the next story (fresh session)
- ❌ Don't expand scope beyond the story
- ❌ Don't modify other stories' files
- ❌ Don't skip verification
- ✅ Do ask if uncertain
- ✅ Do update specs if implementation requires divergence
- ✅ Do flag if estimate is way off (>2x)

---

Begin with Step 0. Tell me which story you're implementing.
