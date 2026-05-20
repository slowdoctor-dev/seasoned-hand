# Story 5.25 — Curator rationale schema versioning + DEBT #92/#93/#94/#96 decisions

> **Status**: done
> **Estimated**: 2.5 hours
> **Dependencies**: 5.17
> **Phase**: 5
> **Type**: backend+docs

---

## Goal

Per F-5.17 + DEBT #96: introduce rationale-versioning compatibility policy + validation tooling
for curator decision payload evolution. Also pin Phase 5 dispositions for DEBT #92 (adaptive
thresholds), #93 (fork-promotion governance — already closed in 5.8), #94 (tiered
retrospective models).

## Acceptance criteria

- [ ] `crate::curator::rationale::SchemaVersion` enum + per-version
      `validate(payload_json) -> Result<...>` helper.
- [ ] CuratorDecision::rationale_json gains an outer `{"schema_version": N, "data": {...}}`
      wrapper (backward-compat with v1 payloads).
- [ ] Architecture amendment paragraph documents the versioning contract.
- [ ] DEBT entries closed:
  - #92 — Phase 5 keeps static thresholds; adaptive remains deferred to Phase 6 with
    explicit metrics gate (close with deferred-to-Phase-6 disposition).
  - #94 — Phase 5 keeps single summarizer profile; tiered-by-size remains deferred to
    Phase 6 with cost/quality threshold criteria (close with deferred-to-Phase-6).
  - #96 — close (this story is the closure).

## Verification

```bash
cargo test -p seasoned-hand-core curator::rationale::schema_version_tests
```

## Refs

- requirements: F-5.15, F-5.16, F-5.17
- architecture: §13 (amendments)
- debt closed: #92 (defer to Phase 6), #94 (defer to Phase 6), #96 (close)
