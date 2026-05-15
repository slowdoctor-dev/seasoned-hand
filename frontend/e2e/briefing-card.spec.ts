// Story 2.24: BriefingCard end-to-end smoke. Drives the
// task_create → briefing_pending → briefing → confirm round-trip
// against a mocked WS, asserting the card renders and the Confirm
// button flips the resolution pill to "Confirmed".
//
// refs: /specs/phase-2/stories/story-2.23.md (the card)
// refs: /specs/phase-2/stories/story-2.24.md (this story)

import { expect, test } from "@playwright/test";
import { MockAgentSocket, miscEvent } from "./helpers/mock-ws";
import { mockProjectsList, mockSessionsList } from "./helpers/fixtures";

test("BriefingCard renders, Confirm flips the pill", async ({ page }) => {
  const mock = new MockAgentSocket();
  await mock.install(page);
  await mockProjectsList(page, []);
  await mockSessionsList(page);

  await page.goto("/");
  await mock.waitOpen();

  // Send a task to mint the session.
  await page.getByPlaceholder("Describe the task...").fill("smoke test goal");
  await page.getByRole("button", { name: "Send" }).click();

  const taskCmd = await mock.waitForCommand(
    (c) => c.payload.cmd === "task_create",
  );
  expect(taskCmd.payload).toMatchObject({
    cmd: "task_create",
    input: "smoke test goal",
  });

  // The chat's onSessionCreated fires from the ack; subscribe follows.
  await mock.waitForCommand(
    (c) =>
      c.payload.cmd === "subscribe" &&
      c.payload.session_id === "mock-session-1",
  );

  await mock.script([
    miscEvent({
      id: "1",
      sessionId: "mock-session-1",
      kindTag: "briefing_pending",
      data: {
        kind: "briefing_pending",
        briefing_call_id: "c1",
        task_id: "t1",
      },
    }),
    miscEvent({
      id: "2",
      sessionId: "mock-session-1",
      kindTag: "briefing",
      data: {
        kind: "briefing",
        briefing_call_id: "c1",
        brief: {
          goal: "Investigate the failing CI run",
          phases: [{ id: 1, title: "Triage", capabilities: ["shell"] }],
          success_criteria: ["root cause identified"],
          expected_deliverables: [
            { filename: "report.md", format: "markdown" },
          ],
        },
      },
    }),
  ]);

  const card = page.getByRole("article", { name: "Briefing" });
  await expect(card).toBeVisible();
  await expect(card.getByText("Investigate the failing CI run")).toBeVisible();
  await expect(card.getByText("report.md")).toBeVisible();

  await card.getByRole("button", { name: "Confirm" }).click();
  const confirmCmd = await mock.waitForCommand(
    (c) => c.payload.cmd === "briefing_confirm",
  );
  expect(confirmCmd.payload).toMatchObject({
    cmd: "briefing_confirm",
    task_id: "t1",
    in_reply_to_call_id: "c1",
    action: "confirm",
  });

  await expect(card.getByText(/^Confirmed/)).toBeVisible();
  // Confirm button is hidden once the card is resolved (mode stays
  // "view" but the footer's Confirm/Edit/Cancel buttons disable).
  await expect(card.getByRole("button", { name: "Confirm" })).toBeDisabled();
});
