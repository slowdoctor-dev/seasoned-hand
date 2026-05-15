// Story 2.24: DecisionsTab smoke. Mints a session via task_create,
// switches to the Decisions tab, scripts one Misc{kind:"decision"}
// WS event, and verifies the row renders + expands to evidence chips.
//
// The decision event uses the flat shape the live server emits today:
// `source`, `reason`, `evidence_event_ids` sit on payload directly
// (decisions-tab.tsx reads `ev.payload`, not `ev.payload.data`).
//
// refs: /specs/phase-2/stories/story-2.22.md (the surface)
// refs: /specs/phase-2/stories/story-2.24.md (this story)

import { expect, test } from "@playwright/test";
import { MockAgentSocket, miscEvent } from "./helpers/mock-ws";
import { mockProjectsList } from "./helpers/fixtures";

test("DecisionsTab renders a row, expands evidence chips", async ({ page }) => {
  const mock = new MockAgentSocket();
  await mock.install(page);
  await mockProjectsList(page, []);

  await page.goto("/");
  await mock.waitOpen();

  await page.getByPlaceholder("Describe the task...").fill("any task");
  await page.getByRole("button", { name: "Send" }).click();
  await mock.waitForCommand(
    (c) =>
      c.payload.cmd === "subscribe" &&
      c.payload.session_id === "mock-session-1",
  );

  // Seed an Action event (referenced by the decision's evidence list)
  // followed by the decision itself.
  await mock.script([
    {
      type: "event",
      id: "5",
      session_id: "mock-session-1",
      ts: 1700_000_000,
      payload: {
        kind: "Action",
        source: "shell",
        args: { cmd: "ls" },
      },
    },
    miscEvent({
      id: "7",
      sessionId: "mock-session-1",
      ts: 1700_000_001,
      kindTag: "decision",
      extra: {
        source: "Verifier",
        reason: "All success criteria passed",
        evidence_event_ids: [5],
      },
    }),
  ]);

  await page.getByRole("button", { name: "Decisions" }).click();

  const row = page.getByRole("button", {
    name: /All success criteria passed/,
  });
  await expect(row).toBeVisible();
  await expect(row.getByText("Verifier", { exact: true })).toBeVisible();
  await expect(row.getByText(/#7 · 1 evidence/)).toBeVisible();

  await row.click();
  await expect(page.getByText("Evidence:", { exact: true })).toBeVisible();
});
