# ADR-008: MIT license, open from day one

Status: Accepted
Date: 2026

## Context

License and visibility decision. Options:

1. **Closed source** — maximize commercial flexibility
2. **Open core / source-available** — community trust + commercial option
3. **MIT** — fully open from day one
4. **Apache 2.0** — fully open with explicit patent grant

## Decision

**MIT license. Public repository from day one.**

No "closed core, open later." No "free for personal, paid for commercial."
No source-available variant. MIT, simple.

## Consequences

**Positive:**
- Maximum community adoption (lowest friction)
- Trust through transparency (audit-friendly)
- Matches Hermes Agent (direct inspiration, MIT-licensed)
- Vendor-neutral: anyone can fork, customize, deploy
- Korean tax law treats MIT contributions cleanly

**Negative:**
- No defensive moat from license terms
- No automatic monetization path
- Anyone can fork and offer competing services
- Patent grant is implicit, not explicit (vs Apache 2.0)

**Neutral:**
- Project credibility depends on execution, not license terms

## Alternatives considered

### Alternative A: Apache 2.0
Explicit patent grant. Slightly more legally robust. But:
- Longer text, more friction for casual readers
- Matches enterprise norms but our audience skews dev/indie
- Patent risks low for this kind of project

Rejected on simplicity. Could relicense to Apache 2.0 later if patent
concerns emerge (MIT → Apache 2.0 is a one-way OK transition).

### Alternative B: AGPL
Strong copyleft. Forces SaaS providers to release modifications. But:
- Reduces enterprise adoption
- "Viral" license scares many users
- Misaligned with the self-hosted thesis

Rejected on adoption.

### Alternative C: Source-available (BSL, Elastic License)
Limit competing SaaS. But:
- Not "open source" by OSI definition
- Confuses contributors
- The community sniff test fails

Rejected on principle.

### Alternative D: Closed source initially
Maximum control. But:
- No early community signal
- Trust is harder to build later
- Hermes/Manus open-source spirit doesn't fit a closed launch

Rejected. Open from day one is part of the value proposition.

## References

- Hermes Agent: MIT
- OpenManus: MIT
- Manus: closed-source SaaS (anti-example)
- choosealicense.com guidance
