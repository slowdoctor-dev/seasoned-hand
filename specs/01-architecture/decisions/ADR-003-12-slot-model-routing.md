# ADR-003: 12-slot model routing pattern

Status: Accepted
Date: 2026

## Context

The agent uses LLMs for many different purposes: agent loop reasoning,
planning, verification, image analysis, web summarization, context
compression, classification, embeddings, etc. These differ wildly in:

- Cost (Claude Opus vs Gemini Flash: 100x difference)
- Required capabilities (tool use, vision, long context)
- Latency tolerance
- Provider availability

Two patterns to choose from:

1. **Single main model** — use one model for everything. Simple, expensive.
2. **Slot-based routing** (Hermes pattern) — different model per task type.

## Decision

Adopt **12-slot routing**: 3 main slots + 9 auxiliary slots.

Main slots: `main`, `planner`, `verifier`
Auxiliary slots: `vision`, `web_extract`, `screenshot`, `compression`,
`session_title`, `session_search`, `classifier`, `embedding`, `reasoning`

Each slot is a 3-tuple: `(provider, model, base_url)`. Special values:
- `provider: auto` — fall back to main if compatible
- `provider: main` — explicit reuse of main slot
- `base_url: <url>` — override provider; any OpenAI-compatible endpoint

## Consequences

**Positive:**
- 50-100x cost reduction for auxiliary tasks (Flash vs Opus)
- Users can mix providers freely (Anthropic main + Gemini auxiliary + local
  embeddings)
- Capability matching: vision slot must support vision; verified at startup
- Self-hosting friendly: any slot can point at local Ollama/vLLM

**Negative:**
- More complex configuration than single-model
- 12 slots is more than most users will tune; documentation needed
- Capability detection complexity (need to query `/v1/models`)

**Neutral:**
- Slot abstraction means swapping providers is config-only (no code change)

## Alternatives considered

### Alternative A: Single main model
Simplest. But:
- Wastes money (Opus running classifier tasks)
- Forces one provider's strengths on every task type
- Vendor lock-in by accident

Rejected on cost and flexibility.

### Alternative B: Provider-based routing (no slots)
"Anthropic for X, OpenAI for Y." But:
- Couples task type to provider identity
- Users can't override without code changes
- Provider goes down = entire task type breaks

Rejected on coupling.

### Alternative C: Dynamic routing based on task complexity
Auto-detect difficulty, pick model accordingly. But:
- Difficult to predict task complexity ahead of time
- Adds latency for the routing decision itself
- Less predictable cost (users prefer fixed-cost-per-task-type)

Rejected as premature optimization. Can layer on top of slots later.

## References

- Hermes Agent slot pattern: https://github.com/NousResearch/hermes
- Capability detection: `/specs/01-architecture/ARCHITECTURE.md` § 3
- Bifrost routing: ADR-001
