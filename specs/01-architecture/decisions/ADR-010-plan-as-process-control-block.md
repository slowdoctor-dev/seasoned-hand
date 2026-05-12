# ADR-010: Plan as Process Control Block (PCB)

Status: Accepted
Date: 2026

## Context

The agent loop has been described as "think → act → observe → repeat." But
in practice, long tasks (50+ iterations) suffer from a phenomenon Manus
calls **goal drift**: the agent gets so absorbed in a sub-problem that it
forgets the original objective.

Without an explicit, structured planning artifact:
- The plan exists only as conversation context, vulnerable to compression
- "Where am I in the task?" becomes implicit (the LLM guesses)
- Course corrections happen ad-hoc, not as a first-class operation
- Tool selection isn't anchored to the current milestone

Manus's solution (confirmed in direct Q&A): a `plan` tool whose state is
a structured object treated as the OS metaphor's **Process Control Block (PCB)**.

## Decision

Treat the plan as a first-class structured artifact, not a free-text
field. Implement a **Plan Manager** component as part of the agent
runtime.

### Structure

```typescript
Plan {
  id: string                      // plan UUID
  session_id: string              // foreign key
  goal: string                    // one-sentence final objective
  phases: Array<Phase>
  current_phase_id: number        // pointer
  created_at: timestamp
  updated_at: timestamp
}

Phase {
  id: number                      // sequential, 1-indexed
  title: string                   // human-readable milestone
  capabilities: Array<string>     // capability flags (e.g., "research", "code")
  status: "pending" | "active" | "done" | "skipped"
}
```

### Storage

```sql
CREATE TABLE plans (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  goal TEXT NOT NULL,
  phases TEXT NOT NULL,        -- JSON array
  current_phase_id INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  FOREIGN KEY (session_id) REFERENCES sessions(id)
);

CREATE INDEX idx_plans_session ON plans(session_id);
```

### Actions (tools)

Three plan-related tools exposed to the agent:

- `plan_create(goal, phases[])` — initial plan at task start
- `plan_advance()` — move `current_phase_id` to next phase (atomic)
- `plan_update(phases[])` — replace phases array (course correction)

### Sticky context injection

The plan is **always** at the top of context for every agent iteration.
This makes it survive context compression (auxiliary `compression` slot
must preserve the plan verbatim).

```rust
async fn build_iteration_context(session_id: &str) -> Context {
    let plan = plan_manager.get_current(session_id).await?;
    Context::new()
        .with_sticky(format_plan(plan))  // ALWAYS top
        .with_events(recent_events(session_id, 100))
        .with_compressed_history(older_events_summary)
        .build()
}
```

## Consequences

**Positive:**
- Goal drift prevented: agent always knows "I'm in phase N of M"
- Tool selection can be filtered by current phase's `capabilities`
- Course corrections are first-class: `plan_update` is auditable
- Plan survives context compression (sticky)
- OS metaphor becomes precise: plan IS the PCB
- Pause/resume becomes trivial: plan + event stream restore full state

**Negative:**
- Adds a new persistence concept (plans table)
- Adds 3 new tools (plan_create / plan_advance / plan_update)
- Every iteration's context grows by plan size (typically 200-500 tokens)
- Requires planner slot model to produce structured JSON output

**Neutral:**
- Plan structure is conventional; doesn't constrain how phases relate
- `capabilities` flag is advisory, not enforced

## Alternatives considered

### Alternative A: Free-text plan in agent messages
Lighter weight. No new tools. But:
- Vulnerable to context compression (could be summarized away)
- "Current phase" inference is unreliable
- No course-correction primitive — agent must regenerate full plan

Rejected on durability grounds.

### Alternative B: Plan as event stream entries
Plan changes are just `Plan` event types in the existing stream. No new
table. But:
- Reading "current plan" requires replaying events
- Sticky context injection harder (need to compute latest state)
- Doesn't capture the "active artifact" nature of the plan

Rejected on access pattern.

### Alternative C: External plan file (markdown in workspace)
Agent writes `plan.md`. Reads it each iteration via file_read. But:
- Requires file_read on every iteration (waste)
- Format unconstrained (drift over time)
- Can't easily query/index for UI display

Rejected on structure and overhead.

### Alternative D: Hierarchical plan (phases with sub-phases)
Manus does not appear to support this. Premature complexity. Defer to a
future ADR if real need emerges.

## Relationship to other decisions

- **ADR-002** (Rust + TS hybrid): Plan Manager is a Rust component in the
  control plane.
- **ADR-005** (SQLite + Redis): `plans` table lives in SQLite, plan updates
  publish to Redis pub/sub for live UI updates.
- **ADR-007** (Conservative learning): Successful plans become candidate
  data for playbook extraction (Phase 3+).
- **Future ADR-NNN** (capabilities flag integration with 12-slot routing):
  Phase 1 will decide how `capabilities` flags map to slot selection.

## References

- Manus Q&A bundle 2, "Plan tool" answer (this conversation)
- OS Process Control Block: https://en.wikipedia.org/wiki/Process_control_block
- ARCHITECTURE.md § (to be added) — Plan Manager component
- Context Engineering 6 principles (PRINCIPLES.md #16, sticky context)
