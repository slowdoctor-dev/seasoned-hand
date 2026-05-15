// Story 2.24: Playwright bootstrap for Phase 2 smoke coverage. Pins
// dev-server port 3001 to avoid clashing with the Rust control plane
// at :3000, which the WS mock helper still pretends to host so the
// HomeShell's `ws://${hostname}:3000/ws` fallback stays addressable.
//
// refs: /specs/phase-2/stories/story-2.24.md
// closes: /specs/phase-1/DEBT.md #9

import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: "http://localhost:3001",
    trace: "retain-on-failure",
  },
  webServer: {
    // package.json's `dev` script already binds to ${PORT:-3001}, so
    // just invoke it. The spec's `pnpm dev -- --port 3001` example is
    // wrong twice over: Next 16's `next dev` doesn't accept `--`
    // pass-through, and the dev script already pins the port.
    command: "pnpm dev",
    port: 3001,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
  projects: [{ name: "chromium", use: { browserName: "chromium" } }],
});
