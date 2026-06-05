# Story 6.6 — Cutover: remove Next.js, compiled Tailwind, infra/docs

> **Status**: ready

Once parity (6.4) and web+desktop targets (6.5) are verified, retire the Next.js
frontend.

## Acceptance criteria

- [ ] Replace the Tailwind CDN (`index.html`) with the compiled Tailwind v4
      pipeline for the Dioxus crate (parity with the old build).
- [ ] `docker-compose.yml`: replace the `frontend` (Node/Next) service with a
      static-serve of the `dx build` web bundle (or have the Rust server serve it).
- [ ] `justfile`: `dev-frontend` removed; `dev-ui` / `build-ui` are canonical.
- [ ] `docs/getting-started.md` + README updated for the Dioxus stack and `dx`.
- [ ] Delete `frontend/` (Next.js app) and its `pnpm` tooling.
- [ ] ARCHITECTURE.md §1.1 already reflects the Dioxus frontend (v1.5); confirm
      no remaining Next.js references in specs/docs.
- [ ] `just verify` + `just check-ui` green; app runs end-to-end via compose.
