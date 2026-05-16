// Story 2.24 regression for story 1.18 — VerifierTab renders verdict
// rows with a pass/fail badge and a one-line reason.
//
// refs: /specs/phase-1/stories/story-1.18.md (the surface)
// refs: /specs/phase-2/stories/story-2.24.md (this story)

import { expect, test } from "@playwright/test";
import { MockAgentSocket } from "./helpers/mock-ws";
import {
  mockProjectsList,
  mockSessionDetail,
  mockSessionVerifications,
} from "./helpers/fixtures";

test("VerifierTab renders a verdict row with badge + reason", async ({
  page,
}) => {
  const mock = new MockAgentSocket();
  await mock.install(page);
  await mockProjectsList(page, []);
  // BrowserTab is the default tab on a fresh session; let its
  // getSession call resolve cleanly so it doesn't surface an error.
  await mockSessionVerifications(page, "mock-session-1", [
    {
      id: "v1",
      session_id: "mock-session-1",
      triggered_at_event_id: 12,
      trigger_kind: "task_complete",
      trigger_detail: null,
      verdict: "pass",
      reason: "All success criteria satisfied",
      evidence_event_ids: [],
      suggested_plan_update: null,
      model_id: "claude-sonnet-4-6",
      cost_cents: 0,
      // Microseconds since unix epoch — matches server's
      // `verifier/persistence.rs` `now_micros()` (proposed DEBT #33).
      created_at: 1_700_000_000_000_000,
    },
  ]);
  await mockSessionDetail(page, "mock-session-1", null);

  await page.goto("/");
  await mock.waitOpen();

  await page.getByPlaceholder("Describe the task...").fill("smoke");
  await page.getByRole("button", { name: "Send" }).click();
  await mock.waitForCommand(
    (c) =>
      c.payload.cmd === "subscribe" &&
      c.payload.session_id === "mock-session-1",
  );

  await page.getByRole("button", { name: "Verifier" }).click();

  await expect(
    page.getByText("All success criteria satisfied"),
  ).toBeVisible();
  // Badge is uppercase-styled but its accessible text is the lowercase
  // verdict; assert as exact text.
  await expect(page.getByText("pass", { exact: true })).toBeVisible();
});
