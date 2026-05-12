# ADR-009: Embarrassingly-parallel work via map tool (deferred to Phase 4+)

Status: Proposed (deferred to Phase 4+)
Date: 2026

## Context

Manus uses a `map` tool that spawns up to 2,000 sub-agents in parallel for
"embarrassingly parallel" tasks — work that breaks into independent
sub-tasks operating on different inputs.

Example use case (from Manus direct Q&A):
> "Find the Sustainability Officer for 100 Global Companies"

Without `map`, this would be 100 sequential research operations taking
hours. With `map`, all 100 run simultaneously and aggregate in minutes.

The question: do we build this into Seasoned Hand, and when?

## Decision

**Defer to Phase 4+**. Do not implement in Phase 0-3.

Specification documented now (this ADR) so Phase 4 work has a ready
blueprint.

## Specification (for Phase 4 implementation)

### Input format

```typescript
MapInput {
  inputs: Array<string | object>       // N items to map over
  prompt_template: string              // handlebars: "Research {{input}}..."
  output_schema: JSONSchema            // structured output per sub-agent
  shared_files?: Array<string>         // optional broadcast files
  max_concurrent?: number              // concurrency cap (default 10)
  timeout_per_sub_agent?: number       // seconds (default 300)
}
```

### Execution model

1. **Orchestrator validates** input array length (cap at 100 in Phase 4
   vs Manus's 2,000 — resource-prohibitive for self-hosting)
2. **Cost estimate** computed: `N × auxiliary_slot_cost_per_call`
3. **User confirmation** required if estimate > configurable threshold
4. **Sub-agent spawn**: each gets its own AIO Sandbox container (clone of
   base image)
5. **Shared file broadcast** (if specified): system copies files into
   every sub-sandbox at `/workspace/`
6. **Sub-agents run in parallel**: each with own input, own sandbox, own
   prompt, own output
7. **Strict isolation**: sub-agents do NOT share state with each other or
   with the main agent during execution
8. **Output validation**: each sub-agent output validated against
   `output_schema` before accepting

### Aggregation

- **Collector worker** (Tokio task) gathers sub-agent outputs
- **Result file** generated: JSON or CSV, written to main session's
  workspace
- **Per-sub-agent status** preserved: `success | failed | timeout`
- **Failure reasons** captured in event stream
- **Result path** returned to main agent

### Failure handling

Main agent's decision tree (from Manus pattern):

1. **Retry**: rerun `map` with only failed inputs
2. **Fallback**: handle failures sequentially with standard browser tool
3. **Partial report**: deliver successful results, explain failures

If overall failure rate >20%, notify user before main agent decides
strategy.

### Required capabilities

- Auxiliary slot must support **JSON mode / structured output**
- (Sub-agents use a cheaper auxiliary slot, NOT main slot — cost control)
- Sandbox manager must support **dynamic container lifecycle**
  (covered by Phase 0 infrastructure decisions)

### Hard limits

- Max sub-agents per map call: **100** (Phase 4)
- Max concurrent sub-agents: **10** (resource cap)
- Max per-sub-agent timeout: **5 minutes**
- Max shared file size: **10MB**
- Per-call cost cap: configurable, default $5

## Consequences

**Positive (when shipped):**
- Unblocks "human-month → minutes" workflows (Manus's claim)
- Differentiation: most OSS agents can't do this
- Sandbox infrastructure already supports the lifecycle (ADR-004)

**Negative (deferring):**
- Users wanting bulk research must do sequential or use external tools
  until Phase 4
- Some marketing positioning loses "wide research" until Phase 4+

**Neutral:**
- Phase 0-3 design should remain compatible with future map addition
  (no architectural changes needed when we add it)

## Rationale for deferral

- Phase 0-3 core value is **depth + learning**, not breadth
- Map adds complexity (sub-agent lifecycle, partial failure decisions,
  cost capping) without supporting core differentiation
- Most user tasks (Phase 0-3 target) don't need 100-way parallelism
- Risk of premature complexity: building map before depth + learning
  works would distract from MVP

## Phase 0 implications (compatibility)

To keep Phase 4 implementation cheap, Phase 0 must ensure:

- **Sandbox manager**: supports starting/stopping multiple containers
  concurrently per session (already designed in ADR-004)
- **Tool dispatcher**: supports dynamic tool registration (so map can be
  added without core refactor)
- **Event stream**: schema can represent sub-task events (extend `source`
  field with `sub_agent_N` prefix)
- **Cost tracking**: per-tool-call cost recording (extends to per-
  sub-agent in Phase 4)

These are all already in scope; no Phase 0 changes required.

## Alternatives considered

### Alternative A: Implement in Phase 0
Match Manus capabilities sooner. But:
- Distracts from depth + learning core
- Premature optimization (most users won't use it Phase 0-3)
- Complexity multiplies (sub-agent failure handling, cost capping)

Rejected on scope discipline.

### Alternative B: Never implement
Skip entirely. But:
- Real value for legitimate use cases (bulk research)
- Manus has demonstrated it's not just marketing
- Defers an option for no good reason

Rejected; defer-with-spec is better than rejecting outright.

### Alternative C: Different parallelism model (e.g., goroutine-style fan-out without isolation)
Lighter weight. No sub-sandboxes. But:
- Loses the "cascading errors prevention" Manus emphasizes
- Sub-tasks can interfere with each other (shared sandbox state)
- Worse predictability

Rejected on safety grounds. Isolation by design matters.

## References

- Manus Q&A bundle 1 + follow-up 1 ("map tool walkthrough")
- ADR-004 (sandbox per session — extends naturally to per sub-agent)
- ARCHITECTURE.md § (Phase 4 section, to be added when we get there)
