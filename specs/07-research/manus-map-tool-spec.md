# Manus's Map Tool — Direct Specification

> Source: direct Q&A with Manus, 2026.
> Used as basis for our ADR-009 (Map tool deferred to Phase 4+).

---

## Manus's example scenario

> "Find the Sustainability Officer for 100 Global Companies"

## Manus's input format

> "Inputs: An array of 100 strings: ['Apple', 'Microsoft', 'Toyota', ...]
> Prompt Template: A specific instruction for the sub-agents:
>   'Research the current Chief Sustainability Officer (CSO) for
>   {{input}}. Find their name, LinkedIn profile URL, and the date they
>   started the role.'
> Output Schema: I define exactly what data I want back:
>   - cso_name (string)
>   - linkedin_url (string)
>   - start_date (string)
>   - source_url (string)"

## Manus on isolation

> "Sub-agents do NOT share state. Each of the 100 sub-agents gets its
> own temporary, isolated environment. Sub-agent #5 (Microsoft) cannot
> see what Sub-agent #6 (Toyota) is doing."

> "Isolation prevents 'cross-contamination.' If one sub-agent crashes or
> gets stuck on a weird website, it doesn't affect the other 99."

## Manus on shared files

> "If I need them to analyze a specific document (e.g., a 500-page PDF),
> I must wrap the file path in <file> tags in the prompt. The system
> then automatically copies that file into all 100 sub-sandboxes."

## Manus on aggregation

> "It generates a single JSON or CSV file (e.g., sustainability_officers.json)
> and saves it to my main sandbox. The tool returns the path to this file
> to me. I then read the file to see the final results."

## Manus on failure handling

Three strategies the main agent can apply:

> 1. Retry: If the failures were due to a temporary glitch, I might run
>    a second `map` call specifically for those 50 failed inputs.
> 2. Fallback: I might try to solve those 50 manually (sequentially)
>    using my standard browser tool to see why they failed.
> 3. Report: If the data simply doesn't exist, I'll present the 950
>    successful results to you and explain why the other 50 could not
>    be found.

## Manus on scale

> "I can spawn up to 2,000 sub-agents simultaneously."

## Manus on value claim

> "This architecture allows me to achieve human-month levels of work in
> a matter of minutes, while maintaining the precision of a structured
> database."

---

## Our adaptation (ADR-009 — deferred)

See `/specs/01-architecture/decisions/ADR-009-map-tool-deferred.md`.

Key differences from Manus:
- Cap at 100 sub-agents (Phase 4), not 2,000 (resource-prohibitive for
  self-hosting)
- Concurrency cap at 10 simultaneous (resource limit)
- Deferred to Phase 4+ to keep Phase 0-3 focused on depth + learning
- Spec written now so Phase 4 implementation has ready blueprint
