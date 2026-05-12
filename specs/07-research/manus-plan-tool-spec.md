# Manus's Plan Tool — Direct Specification

> Source: direct Q&A with Manus, 2026.
> Used as basis for our ADR-010 (Plan as PCB).

This document captures Manus's own description of its plan tool, in its
own words. We extract the spec and synthesize our adaptation in ADR-010.

---

## Manus's structure

> "It is a highly structured object. While you see it rendered as a clean
> Markdown list in our conversation, internally it is handled as a
> JSON-like structure with specific fields:
>
> - Goal: A concise, high-level statement of the final objective.
> - Phases: An array of objects, where each object has:
>   - id: A sequential number.
>   - title: A human-readable description of the milestone.
>   - capabilities: A set of flags (e.g., deep_research, web_development)
>     that tell the system which specialized 'sub-routines' or tools
>     might be needed for that specific phase.
> - Current Phase ID: A pointer that tracks exactly where I am in the
>   process."

## Manus's actions

> "If I'm in Phase 2 (Research) and I discover that the user's request
> is impossible as stated, I don't just fail. I use the `plan` tool to
> `update` the remaining phases."

> "Should I call `plan` with the `advance` action to move to the next
> phase?"

Three actions inferred: `create` (implicit at start), `advance`, `update`.

## Manus on sticky context

> "Because it is part of the context that is passed into the LLM at
> every step, it acts as a 'short-term memory anchor.' Even if the
> conversation gets very long, the plan remains at the top of my 'mind'."

## Manus on goal drift

> "Without a structured plan, an AI agent can suffer from 'Goal Drift'—
> where it gets so focused on solving a small technical bug that it
> forgets it was supposed to be writing a whole research paper."

## Manus's OS metaphor self-application

> "In the 'OS' analogy we used earlier, the Plan is the **Process Control
> Block (PCB)**—it tracks the state, priority, and resources of the
> current 'job' to ensure it reaches completion."

---

## Our adaptation (ADR-010)

See `/specs/01-architecture/decisions/ADR-010-plan-as-process-control-block.md`.

Key differences from Manus:
- Storage as separate SQLite `plans` table (Manus: implementation detail
  not exposed)
- Three explicit tools (`plan_create`, `plan_advance`, `plan_update`)
  vs Manus's single `plan` tool with action parameter
- Status flag per phase ("pending"/"active"/"done"/"skipped") not
  mentioned in Manus's description but useful for our UI display
- Sticky context formalized via context builder (Manus describes it as
  implicit prompt structure)
