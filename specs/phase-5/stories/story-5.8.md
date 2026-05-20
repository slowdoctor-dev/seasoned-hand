# Story 5.8 — playbook_shares + visibility_state + curator integration

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 5.4, 5.5
> **Phase**: 5
> **Type**: backend+curator

---

## Goal

Implement the playbook sharing model per architecture §6.2 (Option B confidence-based
auto-share + review queue). `playbook_shares.visibility_state` governs publication. Curator's
ConsolidationEngine consults the share state when surfacing playbooks to other users.

## Acceptance criteria

- [ ] `crate::sharing::playbook::*` service methods analogous to 5.7's SOP surface.
- [ ] Curator high-confidence extraction sets `visibility_state='shared'`; low-confidence stays
      `review` until admin approval via review queue.
- [ ] Matcher only surfaces playbooks where `playbook_shares.visibility_state='shared'` AND
      tenant + role gates pass.
- [ ] DEBT #93 closure note: optional manual-publish-only mode is a runtime flag, not the
      default.

## Verification

```bash
cargo test -p seasoned-hand-core sharing::playbook::tests
cargo test -p seasoned-hand-core curator::sharing_integration
```

## Refs

- requirements: F-5.7
- architecture: §6.2
- debt closed: #93 (close — policy surface; manual-only mode optional)
