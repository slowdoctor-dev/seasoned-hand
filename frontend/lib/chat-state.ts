import type { ServerEvent } from "./ws-types";

export type InputMode =
  | { kind: "task_create" }
  | { kind: "user_response"; call_id: string }
  | { kind: "disabled" };

/**
 * Derive the chat input's mode from a session's recent events.
 *
 * - No events / no session yet → task_create (creates a new session)
 * - Latest Message has ui:"ask" and no later user response → user_response
 * - Otherwise → disabled (the agent is running)
 */
export function deriveInputMode(
  sessionEvents: ServerEvent[],
  sessionId: string | null,
): InputMode {
  if (sessionId === null || sessionEvents.length === 0) {
    return { kind: "task_create" };
  }

  // Scan from end: find the latest assistant message_ask. If a subsequent
  // user_response (user-role Message) exists, the ask has been answered.
  let askCallId: string | null = null;
  for (let i = sessionEvents.length - 1; i >= 0; i--) {
    const e = sessionEvents[i];
    if (e?.payload.kind !== "Message") continue;
    const role = (e.payload as { role?: string }).role;
    const ui = (e.payload as { ui?: string }).ui;
    if (askCallId === null) {
      if (role === "assistant" && ui === "ask") {
        askCallId = String(e.id);
        // Need to know if any later user message answered it; scan forward.
        const answered = sessionEvents
          .slice(i + 1)
          .some(
            (later) =>
              later.payload.kind === "Message" &&
              (later.payload as { role?: string }).role === "user",
          );
        if (answered) return { kind: "disabled" };
        return { kind: "user_response", call_id: askCallId };
      }
    }
  }

  return { kind: "disabled" };
}
