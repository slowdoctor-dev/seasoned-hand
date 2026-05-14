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
  | "Misc"
  // Phase 2 / story 2.9: the channel-framework ChatChannel as a
  // DeliverySink emits a Misc event that the server's `build_payload`
  // reshapes into a dedicated `Deliverable` payload (architecture §4).
  // Reserved here so frontend story 2.22 can render it without an
  // additional ws-types change. See `DeliverableEventPayload` below.
  | "Deliverable";

export type EventPayload = {
  kind: EventKind;
  // Backend serializes the underlying Event.data as the rest of the payload.
  [key: string]: unknown;
};

// Phase 2 / story 2.9: shape of the Deliverable payload emitted by the
// ChatChannel DeliverySink. Frontend story 2.22 will render this as a
// downloadable card in the chat pane.
export type DeliverableEventPayload = {
  kind: "Deliverable";
  deliverable_id: string;
  format: string;
  file_ref: string;
  citations: number[];
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
