# Story 3.3 — Sync extraction orchestration in task-complete path

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 3.2
> **Phase**: 3
> **Type**: backend

---

## Goal

Implement synchronous extraction execution in `task_complete` with conservative trigger
criteria (PASS + tool_calls>=5), hard timeout, staged error telemetry, and non-blocking
completion behavior.

## Acceptance criteria

- [ ] Extraction is called synchronously from task-complete handling.
- [ ] Trigger gate enforces verifier `pass` and `tool_calls >= 5` only.
- [ ] Timeout at 60s emits `playbook_extraction_timeout` and skips write.
- [ ] Non-timeout failures emit `playbook_extraction_error{stage,reason}` and skip write.
- [ ] Task completion never blocks indefinitely and returns normally on extraction skip.

## Non-goals

- Deterministic safety/redaction and quality-floor enforcement.
- Matcher/injection behavior.

---

## Implementation steps

1. Add extraction call site in task-complete flow.
2. Implement stage-tagged error mapping and timeout wrapper.
3. Emit required `Misc` events and unit-test stage coverage.

---

## Verification

```bash
cargo test -p seasoned-hand-core extraction::sync_path
cargo test -p seasoned-hand-core extraction::timeout_and_error_events
```

---

## Refs

- requirements: F-3.1, F-3.7, NFR-3.1
- architecture: §2.1, §4, §8
