# ADR-015: Relicense MIT → Apache License 2.0

Status: Accepted
Date: 2026-05-21
Supersedes: the license clause of [ADR-008](ADR-008-mit-license-open-from-day-one.md)

## Context

[ADR-008](ADR-008-mit-license-open-from-day-one.md) chose MIT and "open from day
one." The "open from day one" half stands unchanged. The license half is revisited
here at the public-release boundary (repo flipped to Public after Phase 5).

ADR-008 explicitly left this door open in its "Alternatives considered →
Alternative A: Apache 2.0" section:

> "Could relicense to Apache 2.0 later if patent concerns emerge
> (MIT → Apache 2.0 is a one-way OK transition)."

The platform is an autonomous-agent runtime with 38+ tools, an LLM gateway, and a
sandbox execution model. As it goes public and invites outside contributors and
downstream commercial deployments, the **explicit patent grant + patent-retaliation
clause** of Apache 2.0 (§3) is worth the slightly longer license text. MIT's patent
grant is only implicit, which is the one gap ADR-008 itself flagged under
"Negative."

## Decision

**Relicense the project under the Apache License, Version 2.0.**

- `LICENSE` now contains the canonical Apache 2.0 text with a
  `Copyright 2026 Seasoned Hand contributors` notice.
- Add a top-level `NOTICE` file (Apache 2.0 §4(d) attribution convention).
- `Cargo.toml` `workspace.package.license` = `"Apache-2.0"` (SPDX identifier).
- `CONTRIBUTING.md` references Apache 2.0 §5 inbound=outbound contribution terms.
- All prose references (README, BASELINE, GLOSSARY, CHANGELOG) updated MIT → Apache-2.0.

The "open source, public from day one, OSI-approved, no source-available variant"
posture from ADR-008 is unchanged — Apache 2.0 is fully OSI-approved and permissive.

## Consequences

**Positive:**
- Explicit patent grant (§3) — downstream users get patent peace of mind;
  closes the one gap ADR-008 flagged.
- Patent-retaliation clause deters patent litigation against the project.
- Enterprise-friendly: Apache 2.0 is the default expectation for infrastructure
  software, which this is.
- Still permissive + OSI-approved — no adoption friction vs MIT in practice.
- `NOTICE` file gives a clean, conventional place for attribution.

**Negative:**
- Longer license text than MIT (mitigated: contributors rarely read the full text;
  the SPDX id + README line carry the signal).
- Source files don't carry per-file Apache headers (we rely on the repo-root
  LICENSE + Cargo SPDX id, which is acceptable but less explicit than per-file
  headers; revisit if a downstream redistributor needs them).

**Neutral:**
- MIT → Apache 2.0 is a recognized, low-risk one-way transition. No prior
  external releases were published under MIT (the repo went public at this
  boundary), so there is no split-license history to reconcile.

## Alternatives considered

### Keep MIT
Rejected: the public-release boundary is exactly when the implicit-patent-grant gap
matters most, and ADR-008 already earmarked Apache 2.0 as the upgrade path.

### Dual-license (MIT OR Apache-2.0, the Rust-ecosystem norm)
Considered. The Rust community commonly dual-licenses. Rejected for now on
simplicity — a single Apache-2.0 license is unambiguous for downstream commercial
and enterprise consumers, which is the audience the patent grant serves. Could
revisit if the project wants to maximize Rust-ecosystem crate-reuse compatibility.

## References

- Apache License 2.0: https://www.apache.org/licenses/LICENSE-2.0
- ADR-008 (original MIT decision, now superseded on the license clause)
- SPDX identifier: `Apache-2.0`
