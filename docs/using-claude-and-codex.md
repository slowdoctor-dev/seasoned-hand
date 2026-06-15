# Using Claude Code + Codex Together

> Two AI coding tools, one project. Shared instructions via AGENTS.md.

## TL;DR

| | Codex CLI | Claude Code |
|---|---|---|
| Reads | `AGENTS.md` (auto) | `CLAUDE.md` → imports `@AGENTS.md` |
| Source of truth | `AGENTS.md` | `AGENTS.md` (via import) |
| Project config | `.codex/config.toml` | `.claude/settings.json` |
| Sandbox | Built-in strict sandbox | Tool-level permissions |
| Multi-agent | Single agent + tools | Subagents, hooks, skills |
| Best for | Quick iteration, sandboxed exec | Complex codebase exploration, multi-file |

## File structure for both tools

```
/AGENTS.md                  ← source of truth (both tools read)
/CLAUDE.md                  ← imports AGENTS.md + Claude-specific
/.codex/
  config.toml               ← Codex profiles
/.claude/
  settings.json             ← Claude settings
  CLAUDE.local.md           ← personal local overrides (gitignored)
```

## When to use which tool

### Use Claude Code for:
- **Story implementation** (Plan mode + multi-file edits)
- **Codebase exploration** (subagents)
- **Architectural questions** (where Anthropic's reasoning excels)
- **Spec writing** (BMAD personas)
- **Long-running tasks** (better at maintaining focus)

### Use Codex for:
- **Quick fixes** (faster cold start)
- **Sandboxed experiments** (built-in strict sandbox)
- **Different perspective** when Claude gets stuck on a problem
- **CI/automation contexts** (good defaults for non-interactive)

### Use both:
- **Parallel work** — Claude on backend, Codex on frontend
- **Cross-verification** — one implements, the other reviews
- **When stuck** — try the other tool, different model may see solution

## Workflow patterns

### Pattern A: Claude primary, Codex for fast iteration

```bash
# Daily work
claude code           # main story execution

# Quick scripting tasks
codex --profile fast  # one-off scripts, experiments
```

### Pattern B: Codex sandbox + Claude review

```bash
# Codex executes in sandbox
codex --profile story
# implement feature

# Claude reviews
claude code
# "Review the implementation of story 0.5"
```

### Pattern C: Verification by other tool

```bash
# Claude implements
claude code
# implement story 0.5

# Codex verifies
codex --profile review  # read-only sandbox
# "Verify story 0.5 matches /specs/phase-0/stories/story-0.5.md"
```

## Settings divergence

**AGENTS.md** — universal rules (95% of instructions)
**CLAUDE.md** — only Claude-specific additions (5%):
- Subagent usage
- Plan mode preferences
- Hook configurations

**.codex/config.toml** — only Codex-specific:
- Sandbox mode
- Approval policy
- Profile definitions

This way: one source of truth + tool-specific tuning where it actually matters.

## Gotchas

### Codex truncates large AGENTS.md
Keep AGENTS.md under 300 lines. Codex silently truncates beyond `project_doc_max_bytes`.

### Claude Code import syntax
Only `@AGENTS.md` (literal path) works. No glob, no relative paths beyond direct file refs.

### Cascading conflicts
Codex reads AGENTS.md from `~/.codex/` (global), project root, current dir.
If you set global preferences in `~/.codex/AGENTS.md`, they may conflict with project AGENTS.md.
Project AGENTS.md (closer to edit location) wins.

### Different default sandbox behavior
- Codex: strict by default (asks before file writes)
- Claude Code: more permissive by default (relies on user oversight)

Configure to match your trust level.

### Don't duplicate instructions
Anti-pattern:
```
CLAUDE.md          ← full instructions
AGENTS.md          ← full instructions  ❌ duplicate
.cursorrules       ← full instructions  ❌ duplicate
```

Correct:
```
AGENTS.md          ← full instructions (source of truth)
CLAUDE.md          ← @AGENTS.md + Claude-specific
.cursor/rules/     ← @AGENTS.md + Cursor-specific (Cursor 0.50+ reads AGENTS.md)
```

## Verification both tools work

```bash
# Codex reads AGENTS.md?
cd /your/project
codex --profile fast
# Ask: "What stack are we using?" → should mention Bifrost/Rust/Dioxus

# Claude Code reads AGENTS.md via CLAUDE.md?
claude code
# Ask: "What stack are we using?" → same answer
```

If answers differ, check the import chain.

## Cost considerations

- Claude Code uses Anthropic models (subscription or API)
- Codex CLI uses OpenAI (ChatGPT Plus subscription includes Codex)

For Seasoned Hand development:
- Anthropic Claude Sonnet 4.x for complex implementation
- OpenAI GPT-5-codex (or whatever Codex supports) for sandbox/scripting
- Both contribute to project, neither becomes bottleneck

## References

- Codex AGENTS.md spec: https://github.com/openai/codex
- Claude Code memory docs: https://docs.claude.com/en/docs/claude-code/memory
- AGENTS.md convention: https://agentsmd.io (if exists) or community
