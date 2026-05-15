// Story 2.24 regression for story 1.19 — 3-track BrowserTab layout
// sanity check. Asserts the noVNC iframe and the Track B / DOM-text
// pane render once the sandbox object materialises.
//
// refs: /specs/phase-1/stories/story-1.19.md (the surface)
// refs: /specs/phase-2/stories/story-2.24.md (this story)

import { expect, test } from "@playwright/test";
import { MockAgentSocket } from "./helpers/mock-ws";
import {
  mockProjectsList,
  mockSessionDetail,
} from "./helpers/fixtures";

test("BrowserTab renders noVNC iframe + 3-track strip label", async ({
  page,
}) => {
  const mock = new MockAgentSocket();
  await mock.install(page);
  await mockProjectsList(page, []);
  await mockSessionDetail(page, "mock-session-1", {
    novnc_url: "http://localhost:9999/novnc",
    ttyd_url: "http://localhost:9999/ttyd",
    api_url: "http://localhost:9999/api",
  });

  await page.goto("/");
  await mock.waitOpen();

  await page.getByPlaceholder("Describe the task...").fill("smoke");
  await page.getByRole("button", { name: "Send" }).click();
  await mock.waitForCommand(
    (c) =>
      c.payload.cmd === "subscribe" &&
      c.payload.session_id === "mock-session-1",
  );

  // Default tab is `browser` for fresh sessions — no click needed.
  const iframe = page.locator('iframe[title="Sandbox browser (noVNC)"]');
  await expect(iframe).toBeVisible();
  await expect(iframe).toHaveAttribute("src", "http://localhost:9999/novnc");

  await expect(page.getByText("Track B · DOM text")).toBeVisible();
  await expect(page.getByRole("button", { name: "Reload" })).toBeVisible();
});
