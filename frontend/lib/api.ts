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
