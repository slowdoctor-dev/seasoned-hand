# Story 3.4 — Extraction safety filters, redaction, quality floor, and cap events

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 3.3
> **Phase**: 3
> **Type**: backend

---

## Goal

Apply layered adversarial filtering and PII redaction in extraction, enforce minimum
quality-floor checks, and emit deterministic input/output cap events.

## Acceptance criteria

- [ ] Prompt-layer refusal guidance and deterministic adversarial scans are implemented.
- [ ] Deterministic redaction baseline covers token/email/phone/IP/key-like patterns.
- [ ] Quality floor rejects drafts below 3 non-trivial steps or 200 chars.
- [ ] Input cap emits `playbook_extraction_input_truncated` with marker insertion.
- [ ] Output cap emits `playbook_extraction_output_capped`; cap-triggered floor fail emits
      `playbook_extraction_rejected{layer:"quality_floor"}`.
- [ ] `playbook_extraction_rejected` and `playbook_extraction_pii_redacted` payload shape
      matches contract.

## Non-goals

- Benchmark gate and matcher ranking.

---

## Implementation steps

1. Implement deterministic scan/redaction modules.
2. Enforce quality-floor validator on extracted procedure body.
3. Add cap logic and event emission ordering tests.

---

## Verification

```bash
cargo test -p seasoned-hand-core extraction::adversarial_filters
cargo test -p seasoned-hand-core extraction::pii_redaction
cargo test -p seasoned-hand-core extraction::quality_floor_and_caps
```

---

## Refs

- requirements: F-3.13, F-3.14, F-3.18, NFR-3.4, NFR-3.5
- architecture: §3 (quality-floor field), §8, §11
