# Story 6.7 — Release-readiness: docs, deploy, CI/CD, community + checklist

> **Status**: ready

The open-source-release half of Phase 6. Also folds in the release-readiness
checklist that gates the public-release tag (see `/specs/06-roadmap/ROADMAP.md`).

## Acceptance criteria

- [ ] Polished docs (English + Korean): getting-started, configuration,
      architecture overview, migration guide.
- [ ] One-command deploy (`docker compose up -d`) brings up the full stack incl.
      the Dioxus web bundle; scenario configs (cloud / hybrid / fully-local).
- [ ] Demo media (GIFs / short video).
- [ ] CI/CD: build + test backend, build the wasm UI (`dx build`), auto-release.
- [ ] Community channel (Discord or GitHub Discussions); LICENSE / CONTRIBUTING /
      CODE_OF_CONDUCT / SECURITY confirmed polished.

## Release-readiness checklist (gates the public-release tag)

- [ ] Performance track sealed — iter-3 **Codex confirm** (Claude half clean).
- [ ] `cargo test --workspace` green on a Docker host (sandbox tests).
- [ ] Doc reconciliation — no spec drift (ROADMAP/BASELINE/README/ARCHITECTURE).
- [ ] All hardening tracks sealed; no new DEBT carry-ins.
- [ ] BASELINE §8 open decisions resolved or explicitly deferred (cloud sandbox
      provider, telemetry opt-in).

## Acceptance

A new developer installs and runs in < 30 minutes.
