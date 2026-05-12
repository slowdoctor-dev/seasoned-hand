# Story 0.27 — Phase 0 integration test (E2E)

> **Status**: done
> **Estimated**: 4 hours
> **Dependencies**: all of 0.1–0.26
> **Phase**: 0
> **Type**: test + final integration
> **Reads first**: `/specs/phase-0/requirements.md` §5 (acceptance criteria for Phase 0), `/specs/phase-0/architecture.md` §11.3 (E2E target)

## Goal

End-to-end test proving the Phase 0 acceptance from requirements §5:
typing **"Find the GitHub stars of FoundationAgents/OpenManus"**
through the WebSocket produces an agent run that calls
`info_search_web` (or `browser_*` once DEBT #19 closes), narrates via
Message events, and returns a digit-containing answer.

Also: close as many remaining DEBT items as feasible in one cleanup
commit, then write a Phase 0 retrospective.

## Acceptance criteria

### E2E test
- [ ] `crates/seasoned-hand-server/tests/e2e_phase0.rs` (new)
- [ ] Spins up Bifrost + Redis via `docker compose up -d`
- [ ] Starts the control-plane binary on an ephemeral PORT
- [ ] Opens a WS to `/ws`, sends `task_create { input: "<the spec line>" }`
- [ ] Drains events until either:
      - a `Message` event with `role: assistant`, `ui: notify`, and a
        `content` containing at least one digit-only token, OR
      - session transitions to `ERROR`/`FINISHED` with `completed=false`
- [ ] Asserts (when answer arrives): at least one `Action` event with
      a sandbox-backed tool name (search or browser), `tool_calls > 0`,
      `cost_cents > 0`
- [ ] Gated `#[ignore]` since it needs real provider keys + Bifrost
      running; CI wires it once Phase 1 CI lands

### Cleanup
- [ ] Close low-severity DEBT items that landed before E2E (e.g., spec
      drift between specs and implementations introduced along the way)
- [ ] `scripts/spec-check.sh` extended to verify the tool catalog
      count matches the registry (close DEBT #4)

### Retrospective
- [ ] `specs/phase-0/RETROSPECTIVE.md` (new) — 1-page summary:
      - what shipped (each story + commit hash)
      - what we deferred (DEBT.md headline counts by severity)
      - what worked (LLM-agnostic spec workflow, two-agent parallelism, etc.)
      - what to fix in Phase 1 (top 3)

## Non-goals

- CI workflow wiring (separate story; tracked in DEBT #14)
- Frontend E2E via Playwright (Phase 1)
- Multi-session concurrency tests (Phase 1)

## Files changed

- `crates/seasoned-hand-server/tests/e2e_phase0.rs` (new)
- `scripts/spec-check.sh` (extend with tool catalog count check)
- `specs/phase-0/DEBT.md` (close resolved items)
- `specs/phase-0/RETROSPECTIVE.md` (new)

## Spec references

- `/specs/phase-0/requirements.md` §5
- `/specs/phase-0/architecture.md` §11.3

## Commit message

```
feat(phase-0): story 0.27 - Phase 0 E2E + cleanup + retrospective

- tests/e2e_phase0.rs: full stack via WS task_create, asserts
  Action/Observation/Message flow, idle + digit-bearing answer or
  clean ERROR/FINISHED-incomplete. #[ignore]'d — needs Bifrost +
  cloud keys; CI wiring is its own story.
- spec-check.sh: tool catalog count match registry (closes DEBT #4)
- DEBT.md: close items resolved along the way
- RETROSPECTIVE.md: Phase 0 1-pager — shipped vs deferred, what
  worked, top 3 for Phase 1

Phase 0 closes here. Phase 1 starts with the deferred list in
DEBT.md + RETROSPECTIVE.md "top 3 for Phase 1".

refs: /specs/phase-0/stories/story-0.27.md
```
