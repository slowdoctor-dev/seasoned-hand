# Phase 4 — Open Questions

Date: 2026-05-18  
Owner: BMAD Analyst pass

This file captures scope-level tradeoffs that must be explicitly resolved in Architect/PM passes.
Each question includes neutral options with pros/cons and a suggested next owner.

## 1. Auto-archive threshold semantics

Context: Phase 4 requires auto-archive for stale/low-signal playbooks, but threshold semantics define
safety profile and operator trust.

### Options

#### A. Fixed global thresholds

Pros:
- Simple implementation and testing
- Easy to explain operationally

Cons:
- Poor fit across heterogeneous projects
- Can over-archive niche but valid playbooks

#### B. Project-level configurable thresholds

Pros:
- Better fit for project variance
- Supports gradual tuning based on local data

Cons:
- More config complexity
- Higher chance of misconfiguration

#### C. Adaptive thresholds from rolling distributions

Pros:
- Automatically fits local behavior shifts
- Potentially best long-term precision

Cons:
- Harder to reason about in incidents
- Requires robust telemetry and safeguards

Suggested next step: Architect resolves baseline policy; PM scopes staged rollout.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.1 (option B).

## 2. Consolidation similarity metric strategy

Context: Duplicate consolidation can be driven by lexical, semantic, or hybrid signals.

### Options

#### A. FTS overlap only

Pros:
- Cheap and deterministic
- Reuses existing Phase 3 infrastructure

Cons:
- Misses semantic duplicates with different wording
- Higher false-positive/false-negative risk

#### B. Embedding similarity only

Pros:
- Better semantic capture
- Stronger against lexical variance

Cons:
- Higher cost
- Less transparent decision explainability

#### C. Two-stage hybrid (FTS candidate -> embedding rerank)

Pros:
- Balances cost and quality
- Constrains embedding volume

Cons:
- More moving parts
- Requires careful threshold calibration

#### D. Hybrid plus structural-step alignment check

Pros:
- Strongest safety against unrelated merges
- Better procedure-aware matching

Cons:
- Highest complexity
- May reduce throughput in early iteration

Suggested next step: Architect resolves with explicit NFR budget alignment.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.2 (option D).

## 3. Consolidation write behavior

Context: When duplicates are merged, resulting write model affects reversibility and auditability.

### Options

#### A. In-place mutate winner playbook

Pros:
- Minimal schema changes
- Simple query model

Cons:
- Weak rollback story
- Hard provenance reconstruction

#### B. Create new revision and mark predecessors superseded

Pros:
- Strong audit trail
- Safer incident recovery

Cons:
- Additional revision management complexity
- Requires revision-aware ranking updates

#### C. Versioned fork with operator promotion

Pros:
- Maximum safety before activation
- Enables review queue naturally

Cons:
- Slower autonomous improvement loop
- More operational overhead

Suggested next step: Architect resolves; PM may stage B then optional C.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.3 (option B).

## 4. Curator cycle trigger control

Context: Curator can run on cron-like cadence, backlog thresholds, or event-driven hooks.

### Options

#### A. Fixed interval only

Pros:
- Predictable runtime
- Easy capacity planning

Cons:
- Backlog can grow between runs
- Misses near-real-time opportunities

#### B. Backlog threshold only

Pros:
- Work proportional to artifact flow
- Better freshness under burst

Cons:
- Can thrash under noisy inputs
- Harder to coordinate weekly retrospectives

#### C. Dual trigger with guardrails

Pros:
- Balanced timeliness and stability
- Covers both quiet and bursty workloads

Cons:
- Needs deterministic arbitration rules
- More testing matrix size

Suggested next step: Architect resolves default; PM ensures regression coverage.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.4 (option C).

## 5. Curator failure isolation boundary

Context: Failure domain may be per item, per batch, or per full cycle.

### Options

#### A. Fail-fast full cycle

Pros:
- Simple transactional logic
- Easier initial debugging

Cons:
- One bad item blocks entire backlog
- Violates Phase 4 robustness intent

#### B. Per-batch isolation

Pros:
- Partial progress under failure
- Lower orchestration complexity than per-item

Cons:
- Batch poisoning still possible
- Requires batch strategy tuning

#### C. Per-decision-unit isolation

Pros:
- Highest resilience
- Best backlog drain under mixed quality inputs

Cons:
- More granular telemetry/retry logic needed
- Complex idempotency handling

Suggested next step: Architect resolves with clear retry semantics.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.5 (option C).

## 6. Retrospective generation cadence

Context: Roadmap asks weekly retrospectives; implementation can be strict weekly or adaptive.

### Options

#### A. Strict weekly calendar run

Pros:
- Clear operational rhythm
- Simple expectations for users

Cons:
- May generate low-signal reports in low-activity weeks
- Inflexible with burst workloads

#### B. Weekly minimum plus activity-triggered extras

Pros:
- Preserves cadence while improving responsiveness
- Better alignment with operational incidents

Cons:
- More configuration/control complexity
- Potential extra cost

#### C. Activity-threshold only

Pros:
- Reports only when signal exists
- Cost efficient

Cons:
- Breaks roadmap "weekly" expectation
- Harder longitudinal comparison

Suggested next step: Architect/PM jointly resolve; maintain roadmap fidelity.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.6 (option B).

## 7. Retrospective model slot choice

Context: Which slot and model profile should generate retrospectives.

### Options

#### A. Planner slot reuse

Pros:
- No new slot behavior
- Existing prompt discipline reusable

Cons:
- Competes with primary planning workloads
- May be mis-tuned for summarization

#### B. Dedicated summarizer profile within existing routing

Pros:
- Better quality/cost tuning
- Isolates planner path

Cons:
- Additional routing/config complexity
- Needs new policy tests

#### C. Tiered model choice by report size

Pros:
- Cost-aware scaling
- Potentially best quality/efficiency blend

Cons:
- More policy branches
- Harder reproducibility

Suggested next step: Architect resolves with NFR-4.6 cost budget.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.7 (option B).

## 8. Work-pattern signal source

Context: Pattern mining can use full event replay or sampled aggregates.

### Options

#### A. Full event-stream replay

Pros:
- Highest fidelity
- Rich temporal context

Cons:
- Higher compute/storage pressure
- Longer cycle time

#### B. Pre-aggregated metrics only

Pros:
- Low compute cost
- Simpler implementation

Cons:
- Loses important sequence information
- Lower pattern quality

#### C. Hybrid replay window + aggregates

Pros:
- Balanced fidelity and cost
- Better scalability path

Cons:
- Complexity in consistency semantics
- More schema surfaces

Suggested next step: Architect resolves representation and lifecycle.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.8 (option C).

## 9. SOP conflict detection algorithm

Context: Conflict detection quality/noise depends on algorithm class.

### Options

#### A. Rule-based textual contradiction heuristics

Pros:
- Deterministic and explainable
- Low cost

Cons:
- Brittle to paraphrase
- Lower recall

#### B. Semantic entailment/contradiction model only

Pros:
- Better paraphrase handling
- Higher potential recall

Cons:
- Costly and probabilistic
- Harder incident debugging

#### C. Rule-first with semantic adjudication

Pros:
- Good precision/cost balance
- Better explainability than pure semantic

Cons:
- Two-layer complexity
- Needs careful tuning

Suggested next step: Architect resolves baseline and audit method.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.9 (option C).

## 10. Skill self-improvement application mode

Context: Improved procedures can be applied in-place, via revision, or via forks.

### Options

#### A. In-place edits with minimal metadata

Pros:
- Fastest loop
- Low schema overhead

Cons:
- Weak rollback and auditability
- Risky under false positives

#### B. Mandatory revision chain

Pros:
- Strong provenance
- Safer rollback and A/B evaluation

Cons:
- Additional rank/selection complexity
- Storage growth

#### C. Fork then promotion

Pros:
- Maximum operator control
- Explicit release gating

Cons:
- Slower autonomy
- Operational queue burden

Suggested next step: Architect resolves with PM sequencing.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.10 (option B).

## 11. Curator and `playbooks.status='archived'`

Context: Phase 3 already has archive status; Curator must align behavior.

### Options

#### A. Reuse existing status without extension

Pros:
- Minimal migration scope
- Simple compatibility

Cons:
- May lack decision metadata needed for Phase 4
- Limited audit richness

#### B. Extend archived semantics with reason/confidence fields

Pros:
- Better explainability and reversal support
- Stronger analytics

Cons:
- Migration and API updates required
- Slight query complexity increase

#### C. Separate curator archive table with projection to status

Pros:
- Clean separation of concerns
- Rich decision history

Cons:
- More joins and synchronization risk
- Higher implementation complexity

Suggested next step: Architect resolves schema strategy.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.11 (option B).

## 12. Telemetry retention vs storage budget

Context: Retaining enough history for tuning competes with local storage constraints.

### Options

#### A. Fixed TTL delete policy

Pros:
- Predictable storage growth
- Easy to reason operationally

Cons:
- Loses long-horizon tuning data
- Coarse-grained control

#### B. Tiered retention (raw -> summarized)

Pros:
- Preserves trends with bounded size
- Better long-term usefulness

Cons:
- Summarization complexity
- Potential information loss

#### C. Configurable per-project retention classes

Pros:
- Flexible for diverse workloads
- Better fit for power users

Cons:
- More config burden
- Harder support/debug

Suggested next step: Architect resolves default + PM docs requirement.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.12 (option B).

## 13. Embedding warm-up policy

Context: Early cycles can be slow/costly if embeddings are cold.

### Options

#### A. Lazy-only embedding generation

Pros:
- No upfront cost
- Simplest implementation

Cons:
- Early query latency spikes
- Inconsistent behavior

#### B. Background warm-up on new artifact batches

Pros:
- Smoother runtime
- Better predictable latency

Cons:
- Upfront cost bursts
- Needs throttling

#### C. Hybrid with budget-aware warm-up window

Pros:
- Balances user latency and spend
- Supports NFR cost guardrails

Cons:
- More policy complexity
- Requires usage forecasting

Suggested next step: Architect resolves with NFR-4.6 constraints.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.13 (option C).

## 14. L2 cross-source verification rollout timing

Context: DEBT #73 expects L2 enforcement but rollout risk is non-trivial.

### Options

#### A. Enforce globally at Phase 4 start

Pros:
- Immediate safety uplift
- Clear policy consistency

Cons:
- Risk of throughput drop
- Might stall under sparse sources

#### B. Feature-flagged phased rollout by artifact class

Pros:
- Lower rollout risk
- Better observability during tuning

Cons:
- Temporary policy inconsistency
- More operational complexity

#### C. Advisory-only first, enforce later

Pros:
- Data-driven threshold calibration
- Minimal disruption early

Cons:
- Delays safety guarantees
- Could linger without enforcement discipline

Suggested next step: Architect/PM align on staged delivery.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.14 (option B).

## 15. Knowledge/Datasource emit conditions

Context: Phase 3 left writer behavior incomplete; Phase 4 must pin emission boundaries.

### Options

#### A. Emit on every successful extraction

Pros:
- Maximum capture completeness
- Simple logic

Cons:
- High noise and storage growth
- More dedup burden

#### B. Emit only above confidence threshold

Pros:
- Better signal-to-noise
- Lower storage/cost pressure

Cons:
- Risk missing rare-but-useful artifacts
- Requires threshold governance

#### C. Two-tier emit (raw staging + promoted canonical)

Pros:
- Preserves data while controlling active corpus quality
- Supports auditability

Cons:
- More schema/process complexity
- Requires promotion logic

Suggested next step: Architect resolves data lifecycle.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.15 (option C).

## 16. Weekly retrospective evidence policy

Context: Retrospectives must be useful but non-hallucinatory.

### Options

#### A. Free-form summary with soft citation guidance

Pros:
- Higher narrative flexibility
- Simpler prompt design

Cons:
- High hallucination risk
- Weak auditability

#### B. Template-bound sections with mandatory citations

Pros:
- Strong guardrails
- Easier automated validation

Cons:
- Less expressive
- May feel rigid for users

#### C. Hybrid template + optional analysis appendix

Pros:
- Balanced readability and rigor
- Supports advanced insights safely

Cons:
- More complex generation pipeline
- Requires robust parser/validator

Suggested next step: Architect resolves; PM defines acceptance harness.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.16 (option C).

## 17. Curator decision explainability format

Context: Operators need to understand why merge/archive/conflict decisions were made.

### Options

#### A. Store numeric scores only

Pros:
- Compact storage
- Easy machine processing

Cons:
- Poor human interpretability
- Harder incident triage

#### B. Store score + evidence snippets

Pros:
- Better explainability
- Good review queue ergonomics

Cons:
- Larger storage footprint
- Potential PII handling complexity

#### C. Store structured rationale object

Pros:
- Richest audit surface
- Future UI-ready format

Cons:
- Schema complexity
- Versioning concerns

Suggested next step: Architect resolves schema shape and retention strategy.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.17 (option C).

## 18. Cross-project curation in Phase 4

Context: Phase 5 is multi-user; question is whether any cross-project behavior exists in Phase 4.

### Options

#### A. Strict project isolation only

Pros:
- Lowest leakage risk
- Simple policy consistency

Cons:
- Misses early shared-pattern insights

#### B. Opt-in experimental cross-project analytics read-only

Pros:
- Early learning potential
- Controlled blast radius

Cons:
- Policy complexity
- Potential privacy confusion

#### C. Full cross-project curation

Pros:
- Maximum learning network effect

Cons:
- Conflicts with Phase 5 boundary discipline
- High policy and schema complexity

Suggested next step: Analyst recommends A or B only for Phase 4.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.18 (option A).

## 19. Curator governance for low-confidence actions

Context: What happens to uncertain decisions determines safety and operator burden.

### Options

#### A. Drop low-confidence actions silently

Pros:
- Minimal queue burden
- Simpler pipeline

Cons:
- Loses potentially valuable improvements
- Poor transparency

#### B. Queue all low-confidence decisions for review

Pros:
- Maximum safety and visibility
- Clear feedback loop

Cons:
- Human workload may scale poorly
- Possible backlog growth

#### C. Sampled review with confidence bands

Pros:
- Scalable oversight
- Statistical quality monitoring

Cons:
- Some bad actions may slip through
- Requires sampling policy design

Suggested next step: Architect + PM resolve throughput/safety balance.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.19 (option C).

## 20. Phase 4 close-out metric strategy

Context: "one month auto-improves" requires CI-operational proxy.

### Options

#### A. Pure CI replay metrics only

Pros:
- Deterministic and automatable
- Easy gate integration

Cons:
- May diverge from real-world usage
- Limited external validity

#### B. CI replay + canary production telemetry slice

Pros:
- Better realism
- Stronger confidence before phase close-out

Cons:
- More coordination complexity
- Harder reproducibility

#### C. Manual review narrative only

Pros:
- Flexible interpretation

Cons:
- Not objective or repeatable
- Fails phase-level rigor expectation

Suggested next step: Architect defines measurement instrumentation; PM pins gate story.
Resolution footer: -> resolved in /specs/phase-4/architecture.md §12.20 (option B).

## Suggested resolution order

1. Questions 2, 3, 4, 5 (core curator mechanics)
2. Questions 8, 9, 10, 11, 15 (data model and decision semantics)
3. Questions 6, 7, 16, 20 (retrospective and close-out measurement)
4. Questions 1, 12, 13, 14, 17, 18, 19 (policy and operations tuning)

## Ownership note

- Analyst pass defines scope and boundary tradeoffs.
- Architect pass resolves technical mechanism and schema/pipeline design.
- PM pass decomposes resolved decisions into story slices with AC/testability.
