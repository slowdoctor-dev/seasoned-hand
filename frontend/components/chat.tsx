"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  BriefingCard,
  type BriefingResolution,
} from "@/components/chat/briefing-card";
import { deriveInputMode } from "@/lib/chat-state";
import type { UseAgentSocket } from "@/lib/ws";
import type { Brief, ServerEvent } from "@/lib/ws-types";

type Props = {
  sessionId: string | null;
  onSessionCreated?: (id: string) => void;
  events: ServerEvent[];
  send: UseAgentSocket["send"];
};

type LocalResolution = { kind: "confirmed" | "cancelled"; at: number };

export function Chat({ sessionId, onSessionCreated, events, send }: Props) {
  const [draft, setDraft] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [stickToBottom, setStickToBottom] = useState(true);
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  // Optimistic resolution side-channel: Confirm/Cancel acks succeed
  // before the server emits any matching Misc echo, so the card needs
  // a client-side record of "we sent this verb" to flip out of
  // pending. Server-driven states (auto_confirmed / superseded) are
  // derived from events directly and don't need to live here.
  const [localResolutions, setLocalResolutions] = useState<
    Map<string, LocalResolution>
  >(() => new Map());

  // Subscribe whenever sessionId changes.
  useEffect(() => {
    if (sessionId === null) return;
    void send({ cmd: "subscribe", session_id: sessionId, from_event_id: 0 });
  }, [sessionId, send]);

  const sessionEvents = useMemo(
    () => events.filter((e) => e.session_id === sessionId),
    [events, sessionId],
  );

  // Story 2.23: index briefing-related Misc events per session so the
  // BriefingCard can resolve its `task_id` (carried only on the
  // briefing_pending sibling event), detect supersedence (a newer
  // briefing for the same task), and surface auto-confirm. The wire
  // shape for Misc is `{kind:"Misc", kind_tag:"briefing", data:{...}}`
  // so the call_id / task_id / brief fields live under payload.data.
  const briefingIndex = useMemo(() => {
    const taskByCall = new Map<string, string>();
    const latestCallByTask = new Map<string, string>();
    const autoConfirmed = new Set<string>();
    for (const ev of sessionEvents) {
      const p = ev.payload as { kind?: string; kind_tag?: string };
      if (p.kind !== "Misc") continue;
      const tag = p.kind_tag ?? p.kind ?? "";
      const data =
        ((ev.payload as { data?: Record<string, unknown> }).data) ?? {};
      const callId =
        typeof data.briefing_call_id === "string" ? data.briefing_call_id : null;
      if (tag === "briefing_pending") {
        const taskId = typeof data.task_id === "string" ? data.task_id : null;
        if (callId && taskId) taskByCall.set(callId, taskId);
      } else if (tag === "briefing") {
        if (callId) {
          const taskId = taskByCall.get(callId);
          if (taskId) latestCallByTask.set(taskId, callId);
        }
      } else if (tag === "briefing_auto_confirmed") {
        if (callId) autoConfirmed.add(callId);
      }
    }
    return { taskByCall, latestCallByTask, autoConfirmed };
  }, [sessionEvents]);

  const onLocalResolve = useCallback(
    (callId: string, kind: "confirmed" | "cancelled") => {
      setLocalResolutions((prev) => {
        const next = new Map(prev);
        next.set(callId, { kind, at: Date.now() });
        return next;
      });
    },
    [],
  );

  const resolveBriefingResolution = useCallback(
    (callId: string, taskId: string | null): BriefingResolution => {
      if (briefingIndex.autoConfirmed.has(callId)) {
        return { kind: "auto_confirmed" };
      }
      const local = localResolutions.get(callId);
      if (local) return local;
      if (taskId !== null) {
        const latest = briefingIndex.latestCallByTask.get(taskId);
        if (latest && latest !== callId) return { kind: "superseded" };
      }
      return { kind: "pending" };
    },
    [briefingIndex, localResolutions],
  );

  const mode = useMemo(
    () => deriveInputMode(sessionEvents, sessionId),
    [sessionEvents, sessionId],
  );

  useEffect(() => {
    if (!stickToBottom) return;
    const el = scrollerRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [sessionEvents, stickToBottom]);

  const onScroll = () => {
    const el = scrollerRef.current;
    if (!el) return;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 64;
    setStickToBottom(nearBottom);
  };

  const onSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const text = draft.trim();
    if (!text || submitting || mode.kind === "disabled") return;
    setSubmitting(true);
    try {
      if (mode.kind === "task_create") {
        const ack = await send({ cmd: "task_create", input: text });
        if (ack.ok && ack.session_id && onSessionCreated) {
          onSessionCreated(ack.session_id);
        }
      } else if (mode.kind === "user_response" && sessionId) {
        await send({
          cmd: "user_response",
          session_id: sessionId,
          in_reply_to_call_id: mode.call_id,
          content: text,
        });
      }
      setDraft("");
    } finally {
      setSubmitting(false);
    }
  };

  const placeholder =
    mode.kind === "task_create"
      ? "Describe the task..."
      : mode.kind === "user_response"
        ? "Reply..."
        : "Agent is running...";

  return (
    <section className="flex h-full flex-col">
      <div
        ref={scrollerRef}
        onScroll={onScroll}
        className="flex-1 overflow-auto p-4"
      >
        {sessionEvents.length === 0 ? (
          <p className="text-sm text-gray-500">
            {sessionId === null
              ? "Type a task below to start a new session."
              : "Waiting for events…"}
          </p>
        ) : (
          <ul className="flex flex-col gap-2">
            {sessionEvents.map((e) => {
              const briefing = extractBriefing(e);
              if (briefing) {
                const taskId =
                  briefingIndex.taskByCall.get(briefing.callId) ?? null;
                const resolution = resolveBriefingResolution(
                  briefing.callId,
                  taskId,
                );
                return (
                  <li key={`${e.session_id}:${e.id}`}>
                    <BriefingCard
                      brief={briefing.brief}
                      briefingCallId={briefing.callId}
                      taskId={taskId}
                      resolution={resolution}
                      send={send}
                      onLocalResolve={onLocalResolve}
                    />
                  </li>
                );
              }
              return (
                <li key={`${e.session_id}:${e.id}`}>
                  <EventRow event={e} />
                </li>
              );
            })}
          </ul>
        )}
      </div>
      <form
        onSubmit={onSubmit}
        className="flex items-center gap-2 border-t bg-white p-3 dark:bg-black"
      >
        <input
          type="text"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder={placeholder}
          disabled={mode.kind === "disabled" || submitting}
          className="flex-1 rounded border border-gray-300 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-100 dark:border-gray-700 dark:disabled:bg-gray-900"
        />
        <button
          type="submit"
          disabled={mode.kind === "disabled" || submitting || draft.trim() === ""}
          className="rounded bg-blue-600 px-4 py-2 text-sm font-medium text-white disabled:cursor-not-allowed disabled:bg-gray-400"
        >
          Send
        </button>
      </form>
    </section>
  );
}

// Story 2.23: peek at a ServerEvent and, if it's a Misc{kind:"briefing"},
// return the parsed Brief + call_id so the Chat scroller can swap the
// row for a BriefingCard. The wire wraps Misc data under payload.data
// (see ws.rs build_payload), so brief / briefing_call_id live there.
function extractBriefing(
  event: ServerEvent,
): { callId: string; brief: Brief } | null {
  const p = event.payload as { kind?: string; kind_tag?: string };
  if (p.kind !== "Misc") return null;
  const tag = p.kind_tag ?? p.kind ?? "";
  if (tag !== "briefing") return null;
  const data = (event.payload as { data?: Record<string, unknown> }).data;
  if (!data || typeof data !== "object") return null;
  const callId =
    typeof data.briefing_call_id === "string" ? data.briefing_call_id : null;
  const brief = data.brief;
  if (!callId || !brief || typeof brief !== "object") return null;
  return { callId, brief: brief as Brief };
}

function EventRow({ event }: { event: ServerEvent }) {
  const p = event.payload;
  switch (p.kind) {
    case "Message": {
      const role = (p as { role?: string }).role;
      const content = (p as { content?: string }).content ?? "";
      const ui = (p as { ui?: string }).ui;
      // Story 1.18: Narrator-emitted Messages (ui:"narrate") render as
      // a lighter inline note threaded with regular messages. Phase 1
      // narration is non-interactive; clicking is a no-op.
      if (ui === "narrate") {
        return (
          <div
            className="px-3 py-1 text-xs italic text-gray-500 opacity-70 dark:text-gray-400"
            aria-label="narration"
          >
            — {content}
          </div>
        );
      }
      const isUser = role === "user";
      return (
        <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
          <div
            className={`max-w-[80%] rounded-2xl px-3 py-2 text-sm ${
              isUser
                ? "bg-blue-600 text-white"
                : "bg-gray-100 text-gray-900 dark:bg-gray-800 dark:text-gray-100"
            }`}
          >
            {ui === "ask" && (
              <p className="mb-1 text-xs uppercase opacity-60">Question</p>
            )}
            {content}
          </div>
        </div>
      );
    }
    case "Observation": {
      const ok = (p as { ok?: boolean }).ok ?? false;
      // Tool name comes from the originating Action's source field which the
      // server includes as part of the payload when forwarding; fall back to
      // "tool" if unknown.
      const source = (p as { source?: string }).source ?? "tool";
      return (
        <pre className="text-xs font-mono text-gray-600 dark:text-gray-400">
          {source} → {ok ? "ok" : "err"}
        </pre>
      );
    }
    case "Misc": {
      const kind = (p as { kind_tag?: string; kind?: string }).kind_tag
        ?? (p as { kind_tag?: string; kind?: string }).kind
        ?? "misc";
      // Story 2.23: the briefing protocol's three Misc tags are
      // subsumed by BriefingCard's state machine (rendered separately
      // in the Chat scroller), so suppress them here to keep the
      // event stream readable. `briefing` itself is replaced by the
      // card; `briefing_pending` only carries the task_id sidecar the
      // card derives from; `briefing_auto_confirmed` flips the card's
      // pill instead of standing alone.
      if (
        kind === "briefing" ||
        kind === "briefing_pending" ||
        kind === "briefing_auto_confirmed"
      ) {
        return null;
      }
      const text = JSON.stringify(p).slice(0, 80);
      return (
        <p className="text-xs italic text-gray-500">
          {kind}: {text}
        </p>
      );
    }
    default:
      return null;
  }
}
