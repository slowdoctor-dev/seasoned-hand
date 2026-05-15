// One-off Phase 2 screenshot harness. Drives each Phase 2 frontend
// surface against the existing mock fixtures, saves PNGs to
// /tmp/sh-screenshots/. NOT a regression test — use the named .spec.ts
// files for that. Skipped from CI; runs only when invoked directly
// (the filename prefix `_` keeps it out of the playwright default glob
// only if pattern is set; here we keep it included so an explicit
// `pnpm test:e2e -g screenshots` run picks it up).
//
// Run with:
//   pnpm test:e2e -g screenshots --reporter=list
// Output:
//   /tmp/sh-screenshots/{01-project-list,02-briefing-pending,
//     03-briefing-confirmed,04-deliverables-tab,05-decisions-tab,
//     06-home-empty}.png

import { test } from "@playwright/test";
import { MockAgentSocket, miscEvent } from "./helpers/mock-ws";
import {
  mockProjectTasks,
  mockProjectsList,
  mockSessionsList,
  mockTaskDeliverables,
} from "./helpers/fixtures";

const OUT_DIR = "/tmp/sh-screenshots";

// Opt-in: set SH_SCREENSHOTS=1 to run. Default `pnpm test:e2e`
// otherwise picks up this file via the e2e/ glob and pays ~80 s for
// PNGs nobody asked for.
test.describe("Phase 2 screenshots", () => {
  test.skip(
    !process.env.SH_SCREENSHOTS,
    "set SH_SCREENSHOTS=1 to generate /tmp/sh-screenshots/*.png",
  );
  test.use({ viewport: { width: 1440, height: 900 } });

  test("01 — home shell empty state", async ({ page }) => {
    const mock = new MockAgentSocket();
    await mock.install(page);
    await mockProjectsList(page, []);
    await mockSessionsList(page);

    await page.goto("/");
    await mock.waitOpen();
    await page.waitForTimeout(300);
    await page.screenshot({
      path: `${OUT_DIR}/06-home-empty.png`,
      fullPage: false,
    });
  });

  test("02 — project list populated", async ({ page }) => {
    const mock = new MockAgentSocket();
    await mock.install(page);

    const projects = [
      {
        id: "p1",
        tenant_id: null,
        title: "Investigate failing CI",
        description: null,
        status: "active" as const,
        created_at: 0,
        updated_at: 0,
      },
      {
        id: "p2",
        tenant_id: null,
        title: "Q3 customer retention report",
        description: null,
        status: "active" as const,
        created_at: 0,
        updated_at: 0,
      },
      {
        id: "p3",
        tenant_id: null,
        title: "Refactor verifier hot path",
        description: null,
        status: "active" as const,
        created_at: 0,
        updated_at: 0,
      },
    ];
    await mockProjectsList(page, projects);
    await mockProjectTasks(page, "p1", [
      {
        id: "t1",
        project_id: "p1",
        tenant_id: null,
        title: "Triage build failure",
        brief: null,
        status: "running" as const,
        expected_due_at: null,
        completed_at: null,
        failure_reason: null,
        parent_task_id: null,
        schedule: null,
        skill_attached_event_id: null,
        created_at: 0,
        updated_at: 0,
      },
      {
        id: "t2",
        project_id: "p1",
        tenant_id: null,
        title: "Inspect deploy logs",
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
      },
    ]);
    await mockSessionsList(page);

    await page.goto("/");
    await mock.waitOpen();
    await page.getByRole("button", { name: /^Investigate failing CI/ }).click();
    await page.waitForTimeout(300);
    await page.screenshot({
      path: `${OUT_DIR}/01-project-list.png`,
      fullPage: false,
    });
  });

  test("03 — briefing card pending + confirmed", async ({ page }) => {
    const mock = new MockAgentSocket();
    await mock.install(page);
    await mockProjectsList(page, []);
    await mockSessionsList(page);

    await page.goto("/");
    await mock.waitOpen();

    await page
      .getByPlaceholder("Describe the task...")
      .fill("Investigate the failing CI run, summarise root cause into a .docx report");
    await page.getByRole("button", { name: "Send" }).click();
    await mock.waitForCommand((c) => c.payload.cmd === "task_create");
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
            goal: "Investigate the failing CI run and produce a root-cause .docx report",
            phases: [
              { id: 1, title: "Triage build failure", capabilities: ["shell", "browser"] },
              { id: 2, title: "Bisect the offending commit", capabilities: ["shell"] },
              { id: 3, title: "Draft summary deliverable", capabilities: ["task_deliver"] },
            ],
            success_criteria: [
              "Root cause identified with commit reference",
              "Reproducer documented",
              "Fix recommendation included",
            ],
            expected_deliverables: [
              { filename: "ci-root-cause.docx", format: "docx" },
              { filename: "fix-plan.md", format: "markdown" },
            ],
          },
        },
      }),
    ]);

    const card = page.getByRole("article", { name: "Briefing" });
    await card.waitFor({ state: "visible" });
    await page.waitForTimeout(200);
    await page.screenshot({
      path: `${OUT_DIR}/02-briefing-pending.png`,
      fullPage: false,
    });

    await card.getByRole("button", { name: "Confirm" }).click();
    await mock.waitForCommand((c) => c.payload.cmd === "briefing_confirm");
    await page.waitForTimeout(300);
    await page.screenshot({
      path: `${OUT_DIR}/03-briefing-confirmed.png`,
      fullPage: false,
    });
  });

  test("04 — deliverables tab", async ({ page }) => {
    const mock = new MockAgentSocket();
    await mock.install(page);

    const project = {
      id: "p1",
      tenant_id: null,
      title: "Investigate failing CI",
      description: null,
      status: "active" as const,
      created_at: 0,
      updated_at: 0,
    };
    const task = {
      id: "t1",
      project_id: "p1",
      tenant_id: null,
      title: "Triage build failure",
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
      format: "docx",
      source_content_path: "deliverables/.source/ci-root-cause.source.md",
      source_content_sha256: "src-sha",
      rendered_content_path: "deliverables/ci-root-cause.docx",
      rendered_content_sha256: "abc123",
      content_size: 18342,
      citations: null,
      provenance_manifest: {},
      created_at: 0,
    };
    await mockProjectsList(page, [project]);
    await mockProjectTasks(page, "p1", [task]);
    await mockTaskDeliverables(page, "t1", [deliverable], "sess-1");

    await page.goto("/");
    await mock.waitOpen();
    await page.getByRole("button", { name: /^Investigate failing CI/ }).click();
    await page.getByRole("button", { name: /^Triage build failure/ }).click();
    await page.getByRole("button", { name: "Deliverables" }).click();
    await page.waitForTimeout(300);
    await page.screenshot({
      path: `${OUT_DIR}/04-deliverables-tab.png`,
      fullPage: false,
    });
  });

  test("05 — decisions tab", async ({ page }) => {
    const mock = new MockAgentSocket();
    await mock.install(page);
    await mockProjectsList(page, []);
    await mockSessionsList(page);

    await page.goto("/");
    await mock.waitOpen();

    // Mint a session via the chat flow (this is the path DecisionsTab
    // needs an active sessionId set on HomeShell; selecting a task
    // from TaskList in mock mode doesn't set it).
    await page
      .getByPlaceholder("Describe the task...")
      .fill("Investigate failing CI");
    await page.getByRole("button", { name: "Send" }).click();
    await mock.waitForCommand(
      (c) =>
        c.payload.cmd === "subscribe" &&
        c.payload.session_id === "mock-session-1",
    );

    // Decision events use the flat `source`/`reason`/`evidence_event_ids`
    // shape at payload top-level (DecisionsTab reads ev.payload directly,
    // not ev.payload.data).
    await mock.script([
      {
        type: "event",
        id: "10",
        session_id: "mock-session-1",
        ts: 1700_000_000,
        payload: {
          kind: "Action",
          source: "shell",
          args: { cmd: "git bisect run cargo test" },
        },
      },
      miscEvent({
        id: "11",
        sessionId: "mock-session-1",
        ts: 1700_000_001,
        kindTag: "decision",
        extra: {
          source: "Verifier",
          reason: "Triage findings sufficient — proceed to bisection phase",
          evidence_event_ids: [10],
        },
      }),
      miscEvent({
        id: "12",
        sessionId: "mock-session-1",
        ts: 1700_000_002,
        kindTag: "decision",
        extra: {
          source: "plan_advance",
          reason: "Phase 1 (Triage) success criteria met; advancing to Phase 2",
          evidence_event_ids: [10, 11],
        },
      }),
    ]);

    await page.getByRole("button", { name: "Decisions" }).click();
    // Expand the first decision row so evidence chips are visible.
    const firstRow = page.getByRole("button", {
      name: /Triage findings sufficient/,
    });
    await firstRow.waitFor({ state: "visible" });
    await firstRow.click();
    await page.waitForTimeout(300);
    await page.screenshot({
      path: `${OUT_DIR}/05-decisions-tab.png`,
      fullPage: false,
    });
  });
});
