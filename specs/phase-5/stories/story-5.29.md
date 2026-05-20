# Story 5.29 — phase5_event_redaction_visibility_harness + phase5_search_rbac_harness

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: 5.14, 5.15
> **Phase**: 5
> **Type**: test+security

---

## Goal

Two harnesses verifying the load-bearing security carry-forward (DEBT #S-1 from
SECURITY_REVIEW iter-3):

1. `phase5_event_redaction_visibility_harness`: tenant-visible feeds return redacted text;
   admin-only raw route is role-gated.
2. `phase5_search_rbac_harness`: forged tenant or session id in query → zero rows.

## Acceptance criteria

- [ ] Redaction harness seeds events containing PEM keys / Authorization headers / IPv6 /
      emails (the patterns iter-1 security hardening probed) and asserts:
  - tenant-visible read returns `[REDACTED_*]` markers;
  - admin raw read returns original payload AND writes an audit_log row.
- [ ] Search harness asserts query with forged tenant_id returns 0 results AND
      `Misc{kind:"forged_tenant_query_rejected"}` event lands (if enforcement is via app
      layer) or simply 0 rows (if enforced via DB predicate).
- [ ] CI budget < 4 min total.

## Verification

```bash
cargo test -p seasoned-hand-core phase5_event_redaction_visibility_harness
cargo test -p seasoned-hand-core phase5_search_rbac_harness
```

## Refs

- requirements: F-5.11, F-5.12, NFR-5.6
- architecture: §15 harness 4 + 8
- debt closed: #S-1 (verified)
