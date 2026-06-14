// Minimal fetch wrapper for the Rust control plane's /v1 routes.
// Used by frontend components that need REST (TaskList, EditorTab, etc.).

import { authHeaders } from "./auth";

// Single source of truth for the control-plane REST base URL. SSR-safe
// (empty during prerender) and overridable via NEXT_PUBLIC_API_URL.
export const API_BASE =
  typeof window === "undefined"
    ? ""
    : process.env.NEXT_PUBLIC_API_URL ?? `http://${window.location.hostname}:3000`;

export type SessionState =
  | "IDLE"
  | "RUNNING"
  | "FINISHED"
  | "ERROR"
  | "SUSPENDED";

export type SessionSummary = {
  id: string;
  created_at: number;
  updated_at: number;
  state: SessionState;
  title: string | null;
  cost_cents: number;
  tool_calls: number;
};

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, { headers: authHeaders() });
  if (!res.ok) throw new Error(`GET ${path} -> ${res.status}`);
  return (await res.json()) as T;
}

async function postJson<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaders() },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`POST ${path} -> ${res.status}`);
  return (await res.json()) as T;
}

export function listSessions(limit = 50): Promise<SessionSummary[]> {
  return get<SessionSummary[]>(`/v1/sessions?limit=${limit}`);
}

export function getSession(id: string): Promise<SessionSummary & {
  sandbox: { novnc_url: string; ttyd_url: string; api_url: string } | null;
}> {
  return get(`/v1/sessions/${encodeURIComponent(id)}`);
}

// Phase 1 / story 1.9: Verifier verdict DTO — mirrors
// crates/seasoned-hand-core/src/verifier/mod.rs::Verification.
export type Verdict = "pass" | "fail";

export type Verification = {
  id: string;
  session_id: string;
  triggered_at_event_id: number;
  trigger_kind: string;
  trigger_detail: unknown;
  verdict: Verdict;
  reason: string;
  evidence_event_ids: number[];
  suggested_plan_update: unknown | null;
  model_id: string;
  cost_cents: number;
  created_at: number;
};

export type VerificationListResponse = {
  rows: Verification[];
  next_cursor: number | null;
};

export function listVerifications(
  sessionId: string,
  limit = 50,
): Promise<VerificationListResponse> {
  return get<VerificationListResponse>(
    `/v1/sessions/${encodeURIComponent(sessionId)}/verifications?limit=${limit}`,
  );
}

export function getVerification(id: string): Promise<Verification> {
  return get<Verification>(`/v1/verifications/${encodeURIComponent(id)}`);
}

// Phase 2 / story 2.22: project + task + deliverable DTOs.
// Mirrors crates/seasoned-hand-core/src/project/{project,task}.rs and
// crates/seasoned-hand-core/src/deliverable/mod.rs. Hand-mirrored; ts-rs
// codegen stays deferred per phase-2 architecture §5.

export type ProjectStatus = "active" | "archived";

export type Project = {
  id: string;
  tenant_id: string | null;
  title: string;
  description: string | null;
  status: ProjectStatus;
  created_at: number;
  updated_at: number;
};

export type TaskStatus =
  | "drafted"
  | "briefed"
  | "confirmed"
  | "running"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled";

export type Task = {
  id: string;
  project_id: string;
  tenant_id: string | null;
  title: string;
  brief: unknown | null;
  status: TaskStatus;
  expected_due_at: number | null;
  completed_at: number | null;
  failure_reason: string | null;
  parent_task_id: string | null;
  schedule: string | null;
  skill_attached_event_id: number | null;
  created_at: number;
  updated_at: number;
};

export type Deliverable = {
  id: string;
  task_id: string;
  tenant_id: string | null;
  format: string;
  source_content_path: string | null;
  source_content_sha256: string | null;
  rendered_content_path: string;
  rendered_content_sha256: string;
  content_size: number;
  citations: number[] | null;
  provenance_manifest: unknown;
  created_at: number;
};

export type TaskDeliverablesResponse = {
  deliverables: Deliverable[];
  // Latest session_id for the task — used to construct the workspace
  // proxy download URL (`/v1/workspace/:session_id/<path>`). `null` when
  // no session has been created yet.
  latest_session_id: string | null;
};

export function listProjects(limit = 50): Promise<Project[]> {
  return get<Project[]>(`/v1/projects?limit=${limit}`);
}

export function getProject(id: string): Promise<Project> {
  return get<Project>(`/v1/projects/${encodeURIComponent(id)}`);
}

export function createProject(
  title: string,
  description?: string | null,
): Promise<Project> {
  return postJson<Project>("/v1/projects", { title, description });
}

export function listTasks(projectId: string, limit = 50): Promise<Task[]> {
  return get<Task[]>(
    `/v1/projects/${encodeURIComponent(projectId)}/tasks?limit=${limit}`,
  );
}

export function getTask(id: string): Promise<Task> {
  return get<Task>(`/v1/tasks/${encodeURIComponent(id)}`);
}

export function getTaskDeliverables(
  taskId: string,
): Promise<TaskDeliverablesResponse> {
  return get<TaskDeliverablesResponse>(
    `/v1/tasks/${encodeURIComponent(taskId)}/deliverables`,
  );
}

// Phase 2 / story 2.15: provenance manifest accessor — re-used by 2.22
// callers that want intake/brief/sessions context without pulling in
// the full Deliverable row.
export type ProvenanceResponse = {
  deliverable_id: string;
  manifest: unknown;
};

export function getTaskProvenance(taskId: string): Promise<ProvenanceResponse> {
  return get<ProvenanceResponse>(
    `/v1/tasks/${encodeURIComponent(taskId)}/provenance`,
  );
}
