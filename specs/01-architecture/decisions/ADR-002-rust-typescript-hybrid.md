# ADR-002: Rust backend + TypeScript frontend (hybrid)

Status: Accepted
Date: 2026

## Context

We need to choose primary languages. Three patterns considered:

1. **All-TypeScript** — Node.js backend + Next.js frontend. Maximum simplicity.
2. **All-Python** — FastAPI backend + Next.js frontend. AI ecosystem incumbency.
3. **Rust backend + TypeScript frontend** — different language per layer.

Project shape:
- 24/7 agent runtime with 5+ concurrent tasks
- Hot path: agent loop iterations (~50 per task, ~5 tasks = 250 events/min)
- Memory persistence matters for self-hosting on small machines
- Frontend is interactive UI with WebSocket streaming

Performance and safety properties needed:
- True parallel execution (not blocked by single GIL or event loop)
- Predictable memory (no GC pauses, no leaks compounding over weeks)
- Compile-time safety (auto-generated agent code is risky)

## Decision

**Rust backend** (Axum + Tokio + Rig) + **TypeScript frontend** (Next.js 15
+ React 19 + Tailwind v4). Different language per layer.

Rationale: each layer gets the optimal tool. UI is React's domain. Long-
running concurrent runtime is Rust's domain. They communicate via OpenAI-
compatible HTTP and WebSocket.

## Consequences

**Positive:**
- Backend hot path: Rust gives 5x throughput vs Node, 5x memory efficiency
- True parallel concurrency via Tokio (Node single-thread limits removed)
- Compile-time correctness (catches errors AI agents would otherwise introduce)
- Single static binary deployment (no node_modules in production)
- Frontend gets React ecosystem (Next.js, Tailwind, shadcn/ui)

**Negative:**
- Two languages = two toolchains = two skill sets
- Rust learning curve nontrivial for AI-assisted dev (longer compile-fix cycles)
- Type definitions duplicated across boundary (mitigated with `ts-rs` codegen)
- Initial development slower than all-TypeScript

**Neutral:**
- Build complexity goes up slightly (two CI pipelines)
- Communication boundary forces clean API design (could be positive)

## Alternatives considered

### Alternative A: All-TypeScript (Node + Next.js)
Most popular pattern. AI tools are best at TypeScript. But:
- Event-loop blocking under concurrent load
- Memory growth over weeks
- No compile-time correctness for long-running concurrent code
- Limited to single-thread CPU work

Rejected on long-running self-hosted concerns.

### Alternative B: All-Python (FastAPI + Next.js)
AI ecosystem leader. But:
- GIL prevents true parallel execution
- Higher memory footprint
- Recent supply chain incidents (e.g., LiteLLM) raise concerns about
  production deployments depending on the Python interpreter at runtime

Rejected on concurrency and security grounds.

### Alternative C: All-Rust (Tauri-style desktop app)
Maximum performance and safety. But:
- React ecosystem unavailable
- Frontend dev velocity drops significantly
- AI tool support for Rust UIs is weak
- Single-binary deployment vs the standard browser experience

Rejected on frontend velocity grounds.

### Alternative D: Go backend
Good middle ground between Rust and Python. But:
- We're already running Go (Bifrost). Two Go services + frontend feels
  fragmented.
- Rust offers stronger correctness guarantees (Send/Sync types prevent
  data races at compile time).
- Tokio + Rig ecosystem in Rust is mature for our use case.

Rejected as close runner-up. Could revisit if Rust velocity becomes a
sustained problem.

## References

- Bifrost (Go) for LLM gateway: ADR-001
- Rig framework: https://github.com/0xPlaygrounds/rig
- Axum: https://github.com/tokio-rs/axum
- METR 2025 RCT on AI tool speed for compiled languages
- Hermes Agent (Python reference impl) shows performance limits we want to
  avoid
