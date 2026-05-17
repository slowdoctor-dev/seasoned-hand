# Story 3.10 — CLI playbook lifecycle surface

> **Status**: done
> **Estimated**: 1.5 hours
> **Dependencies**: 3.2, 3.5
> **Phase**: 3
> **Type**: cli

---

## Goal

Implement required playbook lifecycle commands for operator control over extracted artifacts.

## Acceptance criteria

- [ ] `seasoned-hand playbook list` returns rows with status/counters.
- [ ] `seasoned-hand playbook show` returns detail for a selected playbook.
- [ ] `seasoned-hand playbook delete` performs soft-delete (`status='archived'`).
- [ ] Archived playbooks are excluded by default from matcher/injection lookup.

## Non-goals

- Playbook edit/export/sharing workflows.

---

## Implementation steps

1. Add `playbook` command group.
2. Wire list/show/delete calls and output formatting.
3. Add soft-delete matcher exclusion tests.

---

## Verification

```bash
cargo test -p seasoned-hand-cli commands::playbook
cargo test -p seasoned-hand-core matcher::exclude_archived
```

---

## Refs

- requirements: F-3.20
- architecture: §2, §4, §6
