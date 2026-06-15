# Story 6.6 — Cutover: remove Next.js, compiled Tailwind, infra/docs

> **Status**: done (issue #5) — core cutover landed; 2 ACs deferred to issue #33.

Once parity (6.4) and web+desktop targets (6.5) are verified, retire the Next.js
frontend. The destructive removal + infra/docs reconciliation shipped in #5; the
compiled-Tailwind pipeline and serving the bundle were split into the #33
follow-up so they don't block the cutover.

## Acceptance criteria

- [ ] **Deferred to #33.** Replace the Tailwind CDN (`index.html`) with the
      compiled Tailwind v4 pipeline for the Dioxus crate (parity with the old build).
- [ ] **Deferred to #33.** `docker-compose.yml`: replace the `frontend` (Node/Next)
      service with a static-serve of the `dx build` web bundle (or have the Rust
      server serve it). The commented Next service was removed in #5; the dx-serve
      decision is the #33 part.
- [x] `justfile`: `dev-frontend` removed; `dev-ui` / `build-ui` are canonical.
- [x] `docs/getting-started.md` + README updated for the Dioxus stack and `dx`.
- [x] Delete `frontend/` (Next.js app) and its `pnpm` tooling.
- [x] ARCHITECTURE.md §1.1 already reflects the Dioxus frontend (v1.5); confirm
      no remaining Next.js references in **live** specs/docs (historical audit/spec
      files — `specs/*_REVIEW.md`, phase-0..5 specs — intentionally left as-is).
- [x] `just verify` + `just check-ui` green (clippy/fmt/wasm; CI mirrors it). The
      end-to-end-via-compose run is part of the #33 dx-serve work.
