# ADR-006: AGENTS.md as universal source of truth

Status: Accepted
Date: 2026

## Context

Multiple AI coding tools exist (Claude Code, Codex CLI, Cursor, Cline,
Windsurf, Aider). Each reads different config files:

- Claude Code → `CLAUDE.md`
- Codex CLI → `AGENTS.md`
- Cursor → `.cursor/rules/` (and 0.50+ reads `AGENTS.md`)
- Cline → various

Without a single source of truth, instructions drift across files.

## Decision

**AGENTS.md is the universal source of truth.** Tool-specific files
(CLAUDE.md, .codex/, .cursor/) contain only tool-specific extensions,
not duplicated content.

CLAUDE.md is a one-line import:
```
See @AGENTS.md for project context.

## Claude Code specific
[only Claude-specific notes]
```

`.codex/config.toml.example` only holds Codex profiles.

## Consequences

**Positive:**
- Switch tools mid-project without instruction drift
- New tool support requires ~5 lines of glue, not duplication
- Codex CLI, Cursor 0.50+, and others read AGENTS.md natively
- Project remains tool-portable as the AI ecosystem evolves

**Negative:**
- AGENTS.md has a 300-line soft limit (Codex truncates beyond
  `project_doc_max_bytes`). Forces concision.
- Tool-specific power features (Claude Plan mode, Codex sandbox profiles)
  need separate documentation

**Neutral:**
- Some tools (e.g., older Cursor versions) need a manual prompt to read
  AGENTS.md

## Alternatives considered

### Alternative A: Tool-specific files everywhere
Each tool has its own full instructions. But:
- Duplication = drift
- Multiplies maintenance N×
- Picking up a new tool requires rewriting

Rejected.

### Alternative B: CLAUDE.md as primary
We use Claude Code most. But:
- Codex doesn't read CLAUDE.md
- Locks the project to one tool

Rejected. ADR-002 already requires multi-tool capability.

### Alternative C: Generic `.aiconfig` or similar new convention
Cleaner but unestablished. But:
- AGENTS.md is the de facto 2026 standard
- Adopting a non-standard name reduces compatibility

Rejected.

## References

- AGENTS.md convention (OpenAI, Sourcegraph, Cursor 0.50+, others)
- `docs/using-claude-and-codex.md` for tool-specific patterns
