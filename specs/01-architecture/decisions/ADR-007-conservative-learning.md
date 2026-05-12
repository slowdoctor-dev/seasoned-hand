# ADR-007: Conservative learning (verified work only)

Status: Accepted
Date: 2026

## Context

Hermes Agent showed that AI can learn from work by extracting skills
(playbooks) from completed tasks. But "learn from work" has two flavors:

1. **Aggressive**: extract from every interaction, every task
2. **Conservative**: extract only from verified, successful, complex tasks

Aggressive learning produces more skills faster, but risks reinforcing
incorrect patterns from failed or unverified work.

## Decision

Learn **conservatively**. A task qualifies for playbook extraction only if:

1. Verifier returned PASS (independent model, fresh context)
2. Task involved ≥5 tool calls (trivial tasks don't merit playbooks)
3. ≥2 similar past tasks exist (pattern stability, not one-off)
4. (Optional) User signaled satisfaction (downloaded result, no rework requested)

Failed tasks never produce learning artifacts. They produce retrospectives
in the event stream, useful for diagnostics but not for replication.

## Consequences

**Positive:**
- Higher-quality playbooks (each is grounded in verified success)
- Lower risk of skill rot
- User trust: "the system only claims to know what it has done well"

**Negative:**
- Slower playbook growth than aggressive policies
- Early-phase users see less compounding benefit
- May miss valuable patterns from imperfect-but-instructive tasks

**Neutral:**
- Curator (Phase 4) compensates by reviewing accumulated playbooks for
  quality and consolidating

## Alternatives considered

### Alternative A: Aggressive learning (every task)
Hermes-style. Most playbooks. But:
- Failed tasks pollute the library
- User loses trust if a learned playbook leads them astray

Rejected on trust grounds.

### Alternative B: User-curated only (no auto-extraction)
No learning unless user manually saves. But:
- Most users won't curate
- Defeats the "seasoned by work" promise

Rejected on user effort.

### Alternative C: Tiered (auto-extract with quarantine)
Auto-extract everything; mark unverified as quarantined; promote only after
verification. But:
- Quarantine adds complexity to data model
- "Quarantined" playbooks may leak into matching queries

Rejected on complexity. Conservative + manual override is simpler.

## References

- `/specs/01-architecture/ARCHITECTURE.md` § 4 (learning trigger logic)
- Phase 3 (learning system) brings this online
- Hermes Agent's learning model (more aggressive than ours)
