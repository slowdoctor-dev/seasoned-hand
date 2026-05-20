# Story 5.22 — Global strict-config harmonization (closes DEBT #91)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 5.2
> **Phase**: 5
> **Type**: refactor+config

---

## Goal

Per F-5.18 + DEBT #91 carry-forward: extend the strict-parse helpers from story 4.14
(`parse_bool_strict`, `parse_u64_strict`, `parse_u32_strict`, `parse_f32_strict`,
`env_*_or_default`) to all non-curator config families. Closes DEBT #91 globally.

## Acceptance criteria

- [ ] Lift the helpers into a shared module (`crate::config::strict` in core, OR keep in
      server crate but expose `pub fn` for CLI reuse).
- [ ] Every `std::env::var("SH_*")` call in main.rs + CLI uses strict parsing.
- [ ] Invalid env values fail-fast at boot with structured error.
- [ ] Pre-existing tests still green; new tests cover non-curator env vars.

## Verification

```bash
cargo test -p seasoned-hand-server strict_config
cargo test -p seasoned-hand-cli strict_config
```

## Refs

- requirements: F-5.18, NFR-5.7
- debt closed: #91 (close)
