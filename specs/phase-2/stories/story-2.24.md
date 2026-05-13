# Story 2.24 — Frontend: Playwright bootstrap + smoke coverage (DEBT #9)

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 2.22, 2.23
> **Phase**: 2
> **Type**: frontend
> **Reads first**: `/specs/phase-1/DEBT.md` #9

---

## Goal

Close Phase 1 DEBT #9 by adding Playwright as a dev dependency and
writing baseline smoke coverage for the three Phase 1 + four Phase 2
frontend surfaces.

## Acceptance criteria

- [ ] `@playwright/test` added to `frontend/package.json` as a
      dev-dependency. `pnpm playwright install chromium` reproducibly
      installs the browser binary.
- [ ] `frontend/playwright.config.ts` configured for `pnpm dev`
      auto-start (web-server reuse) on port 3001 (avoid clashing with
      the Rust server's 3000).
- [ ] `frontend/e2e/` directory with spec files:
      - `briefing-card.spec.ts` — submit a task_create → BriefingCard
        appears → click Confirm → card transitions to confirmed
      - `project-list.spec.ts` — create project → appears in list →
        click project → task list filter changes
      - `deliverables-tab.spec.ts` — stub a deliverable in test
        fixture (mock backend OR seed DB) → Deliverables tab renders
        the row + download link target is correct
      - `decisions-tab.spec.ts` — seed a Misc{kind:"decision"} event
        → Decisions tab renders, click expands evidence chips
      - `chat-narration.spec.ts` (regression for story 1.18) —
        Message{ui:"narrate"} renders with em-dash + italic
      - `verifier-tab.spec.ts` (regression for story 1.18) — verdict
        row renders with badge + reason
      - `browser-tab.spec.ts` (regression for story 1.19) — 3-track
        view layout sanity check
- [ ] Tests use Playwright's network-interception to mock backend
      responses where convenient (project list, deliverables, decisions)
      so they don't need a real Rust server running. The briefing test
      DOES need a real WebSocket exchange — use a local mock WS server
      via Playwright's `page.route` + a small Node WS shim, OR a
      simplified in-tree mock.
- [ ] CI job in `.github/workflows/ci.yml`: new step `frontend-e2e`
      that runs `pnpm playwright install chromium && pnpm playwright
      test` against `pnpm dev`. Stays out of the default matrix if
      flaky; defaults to required if green.
- [ ] `pnpm test:e2e` script added to `frontend/package.json`.
- [ ] `specs/phase-1/DEBT.md` #9 strike-through with this commit's
      SHA.

## Non-goals

- Comprehensive component-level test coverage (Phase 2 ships smoke
  only; full per-component testing is Phase 4+).
- Visual regression / screenshot testing (Phase 4+ if needed).
- Per-PR Playwright runs (Phase 2: workflow_dispatch only; Phase 3+
  can promote to default CI).

---

## Implementation steps

### 1. Install Playwright

```bash
pnpm add -D @playwright/test
pnpm playwright install chromium
```

### 2. playwright.config.ts

```ts
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  webServer: {
    command: "pnpm dev -- --port 3001",
    port: 3001,
    reuseExistingServer: !process.env.CI,
  },
  use: { baseURL: "http://localhost:3001" },
  projects: [{ name: "chromium", use: { browserName: "chromium" } }],
});
```

### 3. Spec files

Each spec uses Playwright's `page.route` to mock backend HTTP responses.
The BriefingCard spec is the trickiest — needs a mock WS or a real
server. Recommended: a tiny `e2e/helpers/mock-ws.ts` that runs a
node WS server on a known port, scripted with canned events.

### 4. CI integration

```yaml
frontend-e2e:
  if: github.event_name == 'workflow_dispatch'   # or push, after stable
  runs-on: ubuntu-latest
  defaults:
    run:
      working-directory: frontend
  steps:
    - uses: actions/checkout@v4
    - uses: pnpm/action-setup@v3
      with: { version: 9 }
    - uses: actions/setup-node@v4
      with:
        node-version: 20
        cache: pnpm
        cache-dependency-path: frontend/pnpm-lock.yaml
    - run: pnpm install --frozen-lockfile
    - run: pnpm playwright install chromium --with-deps
    - run: pnpm test:e2e
```

---

## Verification

```bash
pnpm --dir frontend test:e2e
./scripts/spec-check.sh
```

---

## Files changed

- `frontend/package.json` (modify — devDep + test:e2e script)
- `frontend/pnpm-lock.yaml` (regenerated)
- `frontend/playwright.config.ts` (new)
- `frontend/e2e/*.spec.ts` (new — 7 spec files)
- `frontend/e2e/helpers/mock-ws.ts` (new — for the briefing test)
- `.github/workflows/ci.yml` (modify — new job)
- `specs/phase-1/DEBT.md` (modify — strike-through #9)

---

## Spec references

- `/specs/phase-1/DEBT.md` #9
- `/specs/phase-2/architecture.md` §11 "Frontend (Playwright)"

---

## Commit message

```
test(phase-2): story 2.24 - Frontend Playwright bootstrap + smoke coverage (DEBT #9 close)

- @playwright/test devDep + pnpm playwright install chromium.
- playwright.config.ts: webServer auto-start on port 3001;
  baseURL accordingly.
- 7 spec files covering both Phase 1 surfaces (Chat narrate,
  VerifierTab, 3-track Browser) and Phase 2 additions (BriefingCard,
  ProjectList, DeliverablesTab, DecisionsTab).
- Mocked backend via page.route; mock WS server for the BriefingCard
  confirm round-trip.
- CI workflow_dispatch job frontend-e2e (gated until stable; promoted
  to default ci matrix in a follow-up).
- pnpm test:e2e script.

closes: Phase 1 DEBT #9

refs: /specs/phase-2/stories/story-2.24.md
```

---

## Notes for next story (2.25)

All four Phase-2 frontend surfaces are covered. 2.25 ships the Phase 2
deterministic E2E (50-step + briefing + email roundtrip) on the default
cargo test path.
