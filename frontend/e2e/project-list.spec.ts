// Story 2.24: ProjectList smoke. Mounts the HomeShell with an empty
// projects list, creates a project through the inline form, verifies
// it appears in the left rail and that selecting it switches the
// TaskList header out of the "Select a project" empty state.
//
// refs: /specs/phase-2/stories/story-2.22.md (the surface)
// refs: /specs/phase-2/stories/story-2.24.md (this story)

import { expect, test } from "@playwright/test";
import { MockAgentSocket } from "./helpers/mock-ws";
import {
  mockProjectCreateFlow,
  mockProjectTasks,
} from "./helpers/fixtures";

test("Create + select project surfaces it in TaskList", async ({ page }) => {
  const mock = new MockAgentSocket();
  await mock.install(page);

  const createdProject = {
    id: "p1",
    tenant_id: null,
    title: "Test Project",
    description: null,
    status: "active" as const,
    created_at: 0,
    updated_at: 0,
  };
  await mockProjectCreateFlow(page, {
    initial: [],
    created: createdProject,
  });
  await mockProjectTasks(page, "p1", []);

  await page.goto("/");
  await mock.waitOpen();

  await expect(
    page.getByText("No projects yet. Click", { exact: false }),
  ).toBeVisible();

  await page.getByRole("button", { name: "+ New" }).click();
  await page.getByPlaceholder("Project title").fill("Test Project");
  await page.getByRole("button", { name: "Create" }).click();

  // The new row should appear in the projects list and become active.
  const projectRow = page.getByRole("button", { name: /^Test Project/ });
  await expect(projectRow).toBeVisible();

  // TaskList swaps from the "Select a project" prompt to the project's
  // own empty state once the row is selected. (The Refresh button only
  // renders in the populated-list branch — empty state is heading + the
  // hint text only.)
  await expect(
    page.getByText("Select a project on the left to see its tasks."),
  ).toBeHidden();
  await expect(
    page.getByText("No tasks yet — type one in the center panel."),
  ).toBeVisible();
});
