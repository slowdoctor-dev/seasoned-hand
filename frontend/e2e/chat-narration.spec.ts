// Story 2.24 regression for story 1.18 — Chat narration row renders as
// an em-dashed italic note tagged with `aria-label="narration"`.
//
// refs: /specs/phase-1/stories/story-1.18.md (the surface)
// refs: /specs/phase-2/stories/story-2.24.md (this story)

import { expect, test } from "@playwright/test";
import { MockAgentSocket } from "./helpers/mock-ws";
import { mockProjectsList } from "./helpers/fixtures";

test("Narrate Message renders as a labelled italic note", async ({ page }) => {
  const mock = new MockAgentSocket();
  await mock.install(page);
  await mockProjectsList(page, []);

  await page.goto("/");
  await mock.waitOpen();

  await page.getByPlaceholder("Describe the task...").fill("smoke");
  await page.getByRole("button", { name: "Send" }).click();
  await mock.waitForCommand(
    (c) =>
      c.payload.cmd === "subscribe" &&
      c.payload.session_id === "mock-session-1",
  );

  await mock.scriptOne({
    type: "event",
    id: "1",
    session_id: "mock-session-1",
    ts: 0,
    payload: {
      kind: "Message",
      role: "assistant",
      ui: "narrate",
      content: "thinking about how to start",
    },
  });

  const narration = page.getByLabel("narration");
  await expect(narration).toBeVisible();
  await expect(narration).toContainText("thinking about how to start");
  await expect(narration).toContainText("—");
});
