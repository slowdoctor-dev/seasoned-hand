// Minimal fetch wrapper for the Rust control plane's /v1 routes.
// Used by frontend components that need REST (TaskList, EditorTab, etc.).

const BASE_URL =
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
  const res = await fetch(`${BASE_URL}${path}`);
  if (!res.ok) throw new Error(`GET ${path} -> ${res.status}`);
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
