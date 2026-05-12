# ADR-001: Bifrost as LLM Gateway

Status: Accepted
Date: 2026

## Context

The agent calls LLM APIs constantly — typically 50+ calls per task across
multiple providers (Anthropic, OpenAI, Google, local). We need a gateway
that handles:

- Routing requests to the right provider/model
- Credential pool management (multiple API keys per provider)
- Fallback when primary fails
- Cost tracking
- OpenAI-compatible interface (so our code doesn't depend on provider SDKs)

Three serious options in 2026:

- **LiteLLM** (Python): industry standard, mature, broad provider support
- **Bifrost** (Go): newer, claims significant performance edge
- **Custom-built**: full control, large maintenance burden

Three constraints made the decision urgent:

1. Performance matters. Each tool dispatch waits on the LLM. Gateway overhead
   adds to every iteration.
2. **March 2026 LiteLLM supply chain attack** — backdoored versions 1.82.7
   and 1.82.8 were published to PyPI. Python interpreter model exposes a
   structural attack vector.
3. We're already running multiple compiled-language services (Rust backend,
   Go gateway adds no operational complexity).

## Decision

Use **Bifrost** as the LLM gateway, deployed as a single Docker container.

Configuration via `bifrost/config.yaml`. Our control plane talks to Bifrost
over the OpenAI-compatible HTTP API at `http://localhost:4000/v1`.

## Consequences

**Positive:**
- ~11μs overhead per call vs ~8ms for LiteLLM. Compounds across 50+ calls.
- Single static Go binary, no interpreter loaded at runtime.
- Lower memory footprint (50MB vs ~500MB for LiteLLM).
- OpenAI-compatible API means we can swap to LiteLLM later if needed (low
  switching cost).

**Negative:**
- Bifrost is newer than LiteLLM. Smaller community. Fewer integrations
  proven in production.
- Some advanced LiteLLM features (router strategies, observability hooks)
  may not have direct Bifrost equivalents yet.

**Neutral:**
- We learn Bifrost's config format. Not deeply.

## Alternatives considered

### Alternative A: LiteLLM
Industry standard. Tested. But:
- Python supply chain exposure (March 2026 attack)
- 50-700x higher per-call overhead
- Larger operational footprint

Rejected on performance + security grounds.

### Alternative B: Custom gateway
Full control. But:
- Significant maintenance
- Reinventing solved problems (fallback chains, retry logic, cost tracking)
- Distracts from the actual agent work

Rejected on scope grounds.

### Alternative C: Direct provider SDKs (no gateway)
Simplest. But:
- Our code becomes provider-specific
- No central credential pool
- No fallback chains
- Each model swap = code change

Rejected on agility grounds. The whole point of model-agnostic is to swap
freely.

## References

- Bifrost: https://github.com/maximhq/bifrost
- LiteLLM March 2026 incident discussion
- Hermes Agent's model routing pattern (separately, ADR-003)
- Our `bifrost/config.yaml` template
