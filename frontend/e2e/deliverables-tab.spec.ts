// Story 2.24: DeliverablesTab smoke. Mounts a project with one task,
// switches the right-side AgentComputer to the Deliverables tab, and
// verifies the row renders with its filename, format chip, and a
// Download anchor pointing into the workspace proxy.
//
// refs: /specs/phase-2/stories/story-2.22.md (the surface)
// refs: /specs/phase-2/stories/story-2.24.md (this story)

import { expect, test } from "@playwright/test";
import { MockAgentSocket } from "./helpers/mock-ws";
import {
  mockProjectTasks,
  mockProjectsList,
  mockTaskDeliverables,
} from "./helpers/fixtures";

test("DeliverablesTab renders a row with format chip + download link", async ({
  page,
}) => {
  const mock = new MockAgentSocket();
  await mock.install(page);

  const project = {
    id: "p1",
    tenant_id: null,
    title: "Project One",
    description: null,
    status: "active" as const,
    created_at: 0,
    updated_at: 0,
  };
  const task = {
    id: "t1",
    project_id: "p1",
    tenant_id: null,
    title: "Task One",
    brief: null,
    status: "completed" as const,
    expected_due_at: null,
    completed_at: 0,
    failure_reason: null,
    parent_task_id: null,
    schedule: null,
    skill_attached_event_id: null,
    created_at: 0,
    updated_at: 0,
  };
  const deliverable = {
    id: "d1",
    task_id: "t1",
    tenant_id: null,
    format: "markdown",
    source_content_path: null,
    source_content_sha256: null,
    rendered_content_path: "deliverables/report.md",
    rendered_content_sha256: "abc",
    content_size: 1234,
    citations: null,
    provenance_manifest: {},
    created_at: 0,
  };
  await mockProjectsList(page, [project]);
  await mockProjectTasks(page, "p1", [task]);
  await mockTaskDeliverables(page, "t1", [deliverable], "sess-1");

  await page.goto("/");
  await mock.waitOpen();

  await page.getByRole("button", { name: /^Project One/ }).click();
  await page.getByRole("button", { name: /^Task One/ }).click();
  await page.getByRole("button", { name: "Deliverables" }).click();

  await expect(page.getByText("report.md")).toBeVisible();
  await expect(page.getByText("markdown", { exact: true })).toBeVisible();
  const link = page.getByRole("link", { name: "Download" });
  await expect(link).toBeVisible();
  await expect(link).toHaveAttribute(
    "href",
    /\/v1\/workspace\/sess-1\/deliverables\/report\.md$/,
  );
});
