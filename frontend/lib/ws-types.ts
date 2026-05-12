// Mirrors crates/seasoned-hand-server/src/ws.rs envelope shapes from
// /specs/phase-0/architecture.md §4.2. Hand-written in Phase 0;
// ts-rs codegen deferred to Phase 1 per architecture §2.

export type EventKind =
  | "Message"
  | "Action"
  | "Observation"
  | "Plan"
  | "Knowledge"
  | "Datasource"
  | "Skill"
  | "Misc";

export type EventPayload = {
  kind: EventKind;
  // Backend serializes the underlying Event.data as the rest of the payload.
  [key: string]: unknown;
};

export type ServerEvent = {
  type: "event";
  id: string;
  session_id: string;
  ts: number;
  payload: EventPayload;
};

export type ServerAck = {
  type: "ack";
  id: string;
  ref: string;
  ok: boolean;
  error?: string;
  session_id?: string;
};

export type ServerPing = { type: "ping"; ts: number };
export type ServerPong = { type: "pong"; ts: number };
export type ServerError = {
  type: "error";
  id?: string;
  kind: string;
  message: string;
};

export type ServerEnvelope =
  | ServerEvent
  | ServerAck
  | ServerPing
  | ServerPong
  | ServerError;

export type CommandPayload =
  | { cmd: "subscribe"; session_id: string; from_event_id?: number }
  | {
      cmd: "task_create";
      input: string;
      max_steps?: number;
      cost_cap_cents?: number;
    }
  | { cmd: "task_pause"; session_id: string }
  | { cmd: "task_resume"; session_id: string }
  | { cmd: "task_cancel"; session_id: string }
  | {
      cmd: "user_response";
      session_id: string;
      in_reply_to_call_id: string;
      content: string;
    };

export type ClientCommand = {
  type: "command";
  id: string;
  session_id?: string;
  ts: number;
  payload: CommandPayload;
};

export type ClientPong = { type: "pong"; ts: number };

export type ClientEnvelope = ClientCommand | ClientPong;
