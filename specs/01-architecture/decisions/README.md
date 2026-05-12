# Architecture Decision Records (ADRs)

> Decisions about the system architecture, captured at the point of decision.
> Each ADR has a status (Proposed, Accepted, Deprecated, Superseded) and a date.

---

## Format

Following [Michael Nygard's ADR format](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions):

```
# ADR-NNN: Title

Status: Proposed | Accepted | Deprecated | Superseded by ADR-XXX
Date: YYYY-MM-DD

## Context
What's the situation that forces a decision?

## Decision
What did we decide?

## Consequences
What follows? Both good and bad.

## Alternatives considered
What else did we look at?
```

## Active ADRs

| ID | Title | Status | Date |
|---|---|---|---|
| [ADR-001](ADR-001-bifrost-llm-gateway.md) | Bifrost as LLM Gateway | Accepted | 2026 |
| [ADR-002](ADR-002-rust-typescript-hybrid.md) | Rust backend + TypeScript frontend (hybrid) | Accepted | 2026 |
| [ADR-003](ADR-003-12-slot-model-routing.md) | 12-slot model routing pattern | Accepted | 2026 |
| [ADR-004](ADR-004-aio-sandbox-per-session.md) | AIO Sandbox (Docker) per session | Accepted | 2026 |
| [ADR-005](ADR-005-sqlite-redis-persistence.md) | SQLite WAL + Redis for persistence | Accepted | 2026 |
| [ADR-006](ADR-006-agents-md-source-of-truth.md) | AGENTS.md as universal source of truth | Accepted | 2026 |
| [ADR-007](ADR-007-conservative-learning.md) | Conservative learning (verified work only) | Accepted | 2026 |
| [ADR-008](ADR-008-mit-license-open-from-day-one.md) | MIT license, open from day one | Accepted | 2026 |
| [ADR-009](ADR-009-map-tool-deferred.md) | Map tool (embarrassingly parallel) — deferred to Phase 4+ | Proposed (deferred) | 2026 |
| [ADR-010](ADR-010-plan-as-process-control-block.md) | Plan as Process Control Block (PCB) | Accepted | 2026 |

## Adding a new ADR

1. Copy `template.md` to `ADR-NNN-short-title.md` (next number)
2. Fill in Context, Decision, Consequences, Alternatives
3. Add to the table above
4. Open PR with the ADR for review
5. Mark Accepted (or Proposed pending more info) before merging

## Superseding an ADR

Don't delete old ADRs. Mark them "Superseded by ADR-XXX" and update the
table. History matters for understanding why the system is the way it is.
