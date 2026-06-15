# Story 6.6 — Cutover: remove Next.js, compiled Tailwind, infra/docs

> **Status**: done — core cutover in #5; the Tailwind + serve follow-ups in #33.

Once parity (6.4) and web+desktop targets (6.5) are verified, retire the Next.js
frontend. The destructive removal + infra/docs reconciliation shipped in #5; the
compiled-Tailwind pipeline and serving the bundle were split into the #33
follow-up so they didn't block the cutover — both now landed.

## Acceptance criteria

- [x] **(#33)** Replaced the Tailwind CDN (`index.html`) with a pinned Tailwind v4
      standalone-CLI build (no Node) wired through Dioxus `[web.resource]`; purged
      via `@source` content detection. `just build-css` / `build-ui`.
- [x] **(#33)** The control plane serves the built bundle directly (`SH_UI_DIST` →
      `tower-http` ServeDir + SPA fallback) — single binary, no separate service.
      Chosen over a dedicated compose service (decided with the user).
- [x] `justfile`: `dev-frontend` removed; `dev-ui` / `build-ui` are canonical.
- [x] `docs/getting-started.md` + README updated for the Dioxus stack and `dx`.
- [x] Delete `frontend/` (Next.js app) and its `pnpm` tooling.
- [x] ARCHITECTURE.md §1.1 already reflects the Dioxus frontend (v1.5); confirm
      no remaining Next.js references in **live** specs/docs (historical audit/spec
      files — `specs/*_REVIEW.md`, phase-0..5 specs — intentionally left as-is).
- [x] `just verify` + `just check-ui` green (clippy/fmt/wasm; CI mirrors it). The
      end-to-end-via-compose run is part of the #33 dx-serve work.
