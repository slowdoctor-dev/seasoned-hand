// Story 2.24: shared HTTP-route mocks for the Phase 1/2 frontend
// surfaces. Each helper installs `page.route` interceptors that match
// the Rust control plane's /v1 routes regardless of origin (the
// frontend builds URLs against http://${hostname}:3000 by default —
// see `frontend/lib/api.ts`), so specs run without a backend.
//
// refs: /specs/phase-2/stories/story-2.24.md

import type { Page, Route } from "@playwright/test";
import type {
  Deliverable,
  Project,
  Task,
  Verification,
} from "../../lib/api";

async function fulfillJson(route: Route, body: unknown): Promise<void> {
  await route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify(body),
  });
}

/** Mock `GET /v1/projects` with a fixed list. */
export async function mockProjectsList(
  page: Page,
  projects: Project[],
): Promise<void> {
  await page.route(/\/v1\/projects(?:\?[^/]*)?$/, async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    await fulfillJson(route, projects);
  });
}

/** Mock the full create→list refresh cycle on `POST /v1/projects`. */
export async function mockProjectCreateFlow(
  page: Page,
  args: {
    initial: Project[];
    created: Project;
  },
): Promise<void> {
  let cycle = 0;
  await page.route(/\/v1\/projects(?:\?[^/]*)?$/, async (route) => {
    const req = route.request();
    if (req.method() === "GET") {
      const body = cycle === 0 ? args.initial : [...args.initial, args.created];
      await fulfillJson(route, body);
      return;
    }
    if (req.method() === "POST") {
      cycle++;
      await fulfillJson(route, args.created);
      return;
    }
    await route.fallback();
  });
}

/** Mock `GET /v1/projects/:id/tasks` with a fixed list. */
export async function mockProjectTasks(
  page: Page,
  projectId: string,
  tasks: Task[],
): Promise<void> {
  const pattern = new RegExp(
    `/v1/projects/${escapeRegex(projectId)}/tasks(?:\\?[^/]*)?$`,
  );
  await page.route(pattern, async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    await fulfillJson(route, tasks);
  });
}

/** Mock `GET /v1/tasks/:id/deliverables` with a fixed body. */
export async function mockTaskDeliverables(
  page: Page,
  taskId: string,
  deliverables: Deliverable[],
  latestSessionId: string | null,
): Promise<void> {
  const pattern = new RegExp(
    `/v1/tasks/${escapeRegex(taskId)}/deliverables(?:\\?[^/]*)?$`,
  );
  await page.route(pattern, async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    await fulfillJson(route, {
      deliverables,
      latest_session_id: latestSessionId,
    });
  });
}

/** Mock `GET /v1/sessions/:id/verifications` with a fixed list. */
export async function mockSessionVerifications(
  page: Page,
  sessionId: string,
  rows: Verification[],
): Promise<void> {
  const pattern = new RegExp(
    `/v1/sessions/${escapeRegex(sessionId)}/verifications(?:\\?[^/]*)?$`,
  );
  await page.route(pattern, async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    await fulfillJson(route, { rows, next_cursor: null });
  });
}

/** Mock `GET /v1/sessions/:id` for the BrowserTab sandbox lookup. */
export async function mockSessionDetail(
  page: Page,
  sessionId: string,
  sandbox: { novnc_url: string; ttyd_url: string; api_url: string } | null,
): Promise<void> {
  const pattern = new RegExp(
    `/v1/sessions/${escapeRegex(sessionId)}(?:\\?[^/]*)?$`,
  );
  await page.route(pattern, async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    await fulfillJson(route, {
      id: sessionId,
      created_at: 0,
      updated_at: 0,
      state: "RUNNING",
      title: null,
      cost_cents: 0,
      tool_calls: 0,
      sandbox,
    });
  });
}

/** Mock `GET /v1/sessions` (used by TaskList in archive mode). */
export async function mockSessionsList(page: Page): Promise<void> {
  await page.route(/\/v1\/sessions(?:\?[^/]*)?$/, async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    await fulfillJson(route, []);
  });
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
