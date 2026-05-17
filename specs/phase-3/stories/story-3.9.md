# Story 3.9 — CLI SOP lifecycle surface

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 3.2
> **Phase**: 3
> **Type**: cli

---

## Goal

Implement required SOP CLI authoring and inspection commands in Phase 3.

## Acceptance criteria

- [ ] `seasoned-hand sop create` creates row in `sops`.
- [ ] `seasoned-hand sop edit` updates content/version.
- [ ] `seasoned-hand sop list` lists current SOPs.
- [ ] `seasoned-hand sop show` renders one SOP in detail.
- [ ] `seasoned-hand sop delete` removes targeted SOP row.

## Non-goals

- Frontend SOP editor.
- New LLM-callable SOP write tool.

---

## Implementation steps

1. Add `sop` command group and argument parsing.
2. Bind to server/core API paths or direct client endpoints.
3. Add command tests and help docs.

---

## Verification

```bash
cargo test -p seasoned-hand-cli commands::sop
seasoned-hand sop --help
```

---

## Refs

- requirements: F-3.10
- architecture: §2, §4
