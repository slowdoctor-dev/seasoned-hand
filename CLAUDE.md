# Claude Code Instructions

See @AGENTS.md for project context, architecture, methodology, and conventions.
For session startup: read @BASELINE.md first (single entry point).

---

## Claude Code specific

### Tool usage preferences

- **Subagents** — spawn for codebase exploration when story spans 5+ files
- **Bash tool** — use for `just verify` before declaring story done
- **Edit tool** — prefer over Write for modifying existing files (Write replaces entire file)
- **Glob/Grep** — prefer over recursive Read for searching
- **WebFetch** — use sparingly; prefer reading `/specs/` for project decisions

### Memory hierarchy (Claude Code reads in order)

1. `~/.claude/CLAUDE.md` — your global preferences (personal)
2. `/CLAUDE.md` (this file) — project-level
3. `@AGENTS.md` import — source of truth
4. `.claude/CLAUDE.local.md` — your local-only overrides (gitignored)

### Commands and slash commands

- `/clear` — start fresh between stories (mandatory)
- `/compact` — only if context approaching limit mid-story (rare)
- `/self-review` — before creating PR

### Plan mode

For stories with 3+ acceptance criteria, enter Plan mode first.
Output structured plan, wait for explicit "go" before executing.

### Sub-agents

When story spans frontend + backend + spec:
```
1. Subagent A: implement backend changes
2. Subagent B: implement frontend changes
3. Main agent: update spec, run integration test, commit
```

Each subagent gets fresh context with only its specific spec section.

---

## What NOT to do (Claude-specific additions)

- Don't use Plan mode for trivial 1-acceptance-criteria stories
- Don't spawn subagents for stories under 3 files
- Don't compact mid-story — start fresh story instead
- Don't auto-accept large multi-file edits — review each
