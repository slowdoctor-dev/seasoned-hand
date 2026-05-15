"use client";

// Story 2.23: Briefing card — the user-facing half of story 2.8's
// confirm gate. Renders an authored Brief inline in the Chat panel
// with Confirm / Edit / Cancel actions. Edit toggles a JSON textarea
// that POSTs a PartialBrief via the WS briefing_confirm cmd; the
// server re-emits a new Briefing event with a new briefing_call_id,
// and the older card flips to "Superseded".
//
// Phase 2 ships the JSON textarea editor only — per-field editing is
// Phase 4+ (story 2.23 non-goal). Auto-confirm timeout (5 min) is
// server-driven; the briefing_auto_confirmed Misc event drives the
// card's terminal state in the parent's resolution map.
//
// refs: /specs/phase-2/architecture.md §2.2, §4
// refs: /specs/phase-2/stories/story-2.23.md

import { useMemo, useState } from "react";
import type { Brief, PartialBrief } from "@/lib/ws-types";
import type { UseAgentSocket } from "@/lib/ws";

export type BriefingResolution =
  | { kind: "pending" }
  | { kind: "confirmed"; at: number }
  | { kind: "cancelled"; at: number }
  | { kind: "auto_confirmed" }
  | { kind: "superseded" };

type Props = {
  brief: Brief;
  briefingCallId: string;
  taskId: string | null;
  resolution: BriefingResolution;
  send: UseAgentSocket["send"];
  onLocalResolve: (callId: string, kind: "confirmed" | "cancelled") => void;
};

export function BriefingCard({
  brief,
  briefingCallId,
  taskId,
  resolution,
  send,
  onLocalResolve,
}: Props) {
  const [mode, setMode] = useState<"view" | "edit">("view");
  // Each Briefing Misc event has a unique server event id, so the
  // parent re-keys the card on every new briefing_call_id — meaning
  // the useState initializer captures the right Brief on mount and
  // we never need a synchronization effect.
  const [editJson, setEditJson] = useState(() => JSON.stringify(brief, null, 2));
  const [parseError, setParseError] = useState<string | null>(null);
  const [requestError, setRequestError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  const isResolved = resolution.kind !== "pending";
  const taskKnown = taskId !== null;

  const sendBriefing = async (
    action: "confirm" | "cancel" | "edit",
    edits?: PartialBrief,
  ): Promise<boolean> => {
    if (!taskKnown || taskId === null) {
      setRequestError(
        "Cannot resolve briefing — task id not yet known. Try again in a moment.",
      );
      return false;
    }
    setPending(true);
    setRequestError(null);
    try {
      const ack = await send({
        cmd: "briefing_confirm",
        task_id: taskId,
        in_reply_to_call_id: briefingCallId,
        action,
        ...(edits ? { edits } : {}),
      });
      if (!ack.ok) {
        setRequestError(ack.error ?? "briefing_confirm failed");
        return false;
      }
      return true;
    } catch (err) {
      setRequestError(err instanceof Error ? err.message : String(err));
      return false;
    } finally {
      setPending(false);
    }
  };

  const onConfirm = async () => {
    const ok = await sendBriefing("confirm");
    if (ok) onLocalResolve(briefingCallId, "confirmed");
  };

  const onCancel = async () => {
    const ok = await sendBriefing("cancel");
    if (ok) onLocalResolve(briefingCallId, "cancelled");
  };

  const onSaveEdit = async () => {
    let parsed: PartialBrief;
    try {
      parsed = JSON.parse(editJson) as PartialBrief;
    } catch (err) {
      setParseError(err instanceof Error ? err.message : "invalid JSON");
      return;
    }
    setParseError(null);
    const ok = await sendBriefing("edit", parsed);
    if (ok) {
      // Server will re-emit a new Briefing event; the parent's
      // resolution map flips this card to "superseded" once the new
      // call_id arrives. Drop back to view mode in the meantime.
      setMode("view");
    }
  };

  const onDiscardEdit = () => {
    setEditJson(JSON.stringify(brief, null, 2));
    setParseError(null);
    setRequestError(null);
    setMode("view");
  };

  const containerClass = useMemo(() => {
    const base =
      "rounded-lg border bg-white p-3 text-sm shadow-sm dark:bg-gray-950";
    if (resolution.kind === "superseded") {
      return `${base} border-gray-200 opacity-50 dark:border-gray-800`;
    }
    if (resolution.kind === "cancelled") {
      return `${base} border-red-200 dark:border-red-900`;
    }
    if (resolution.kind === "confirmed" || resolution.kind === "auto_confirmed") {
      return `${base} border-green-200 dark:border-green-900`;
    }
    return `${base} border-blue-200 dark:border-blue-900`;
  }, [resolution.kind]);

  return (
    <article className={containerClass} aria-label="Briefing">
      <header className="mb-2 flex items-baseline justify-between gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-blue-700 dark:text-blue-300">
          Briefing
        </h3>
        <ResolutionPill resolution={resolution} />
      </header>

      {mode === "view" ? (
        <BriefView brief={brief} />
      ) : (
        <BriefEditor
          value={editJson}
          onChange={setEditJson}
          parseError={parseError}
        />
      )}

      {requestError && (
        <p className="mt-2 rounded bg-red-50 px-2 py-1 text-xs text-red-700 dark:bg-red-950 dark:text-red-300">
          {requestError}
        </p>
      )}

      <footer className="mt-3 flex flex-wrap items-center gap-2">
        {mode === "view" ? (
          <>
            <button
              type="button"
              onClick={onConfirm}
              disabled={isResolved || pending || !taskKnown}
              className="rounded bg-blue-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-blue-700 disabled:cursor-not-allowed disabled:bg-gray-400"
            >
              Confirm
            </button>
            <button
              type="button"
              onClick={() => setMode("edit")}
              disabled={isResolved || pending || !taskKnown}
              className="rounded border border-gray-300 px-3 py-1.5 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:cursor-not-allowed disabled:opacity-50 dark:border-gray-700 dark:text-gray-300 dark:hover:bg-gray-900"
            >
              Edit
            </button>
            <button
              type="button"
              onClick={onCancel}
              disabled={isResolved || pending || !taskKnown}
              className="rounded border border-red-300 px-3 py-1.5 text-xs font-medium text-red-700 hover:bg-red-50 disabled:cursor-not-allowed disabled:opacity-50 dark:border-red-900 dark:text-red-300 dark:hover:bg-red-950"
            >
              Cancel
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              onClick={onSaveEdit}
              disabled={pending || !taskKnown}
              className="rounded bg-blue-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-blue-700 disabled:cursor-not-allowed disabled:bg-gray-400"
            >
              Save
            </button>
            <button
              type="button"
              onClick={onDiscardEdit}
              disabled={pending}
              className="rounded border border-gray-300 px-3 py-1.5 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:cursor-not-allowed disabled:opacity-50 dark:border-gray-700 dark:text-gray-300 dark:hover:bg-gray-900"
            >
              Discard
            </button>
          </>
        )}
      </footer>
    </article>
  );
}

function ResolutionPill({ resolution }: { resolution: BriefingResolution }) {
  switch (resolution.kind) {
    case "pending":
      return null;
    case "confirmed":
      return (
        <span className="rounded bg-green-100 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-green-700 dark:bg-green-900 dark:text-green-200">
          Confirmed {formatTime(resolution.at)}
        </span>
      );
    case "cancelled":
      return (
        <span className="rounded bg-red-100 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-red-700 dark:bg-red-900 dark:text-red-200">
          Cancelled {formatTime(resolution.at)}
        </span>
      );
    case "auto_confirmed":
      return (
        <span className="rounded bg-amber-100 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-amber-700 dark:bg-amber-900 dark:text-amber-200">
          Auto-confirmed
        </span>
      );
    case "superseded":
      return (
        <span className="rounded bg-gray-200 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-gray-700 dark:bg-gray-800 dark:text-gray-300">
          Superseded
        </span>
      );
  }
}

function BriefView({ brief }: { brief: Brief }) {
  return (
    <div className="space-y-3 text-gray-900 dark:text-gray-100">
      <p className="text-sm">{brief.goal}</p>

      {brief.phases.length > 0 && (
        <section>
          <h4 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-gray-500">
            Phases
          </h4>
          <ol className="ml-4 list-decimal space-y-0.5 text-xs">
            {brief.phases.map((p) => (
              <li key={p.id}>
                <span className="font-medium">{p.title}</span>
                {p.capabilities && p.capabilities.length > 0 && (
                  <span className="ml-2 text-gray-500">
                    [{p.capabilities.join(", ")}]
                  </span>
                )}
              </li>
            ))}
          </ol>
        </section>
      )}

      {brief.success_criteria.length > 0 && (
        <section>
          <h4 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-gray-500">
            Success criteria
          </h4>
          <ul className="ml-4 list-disc space-y-0.5 text-xs">
            {brief.success_criteria.map((sc, idx) => (
              <li key={idx}>{sc}</li>
            ))}
          </ul>
        </section>
      )}

      {brief.expected_deliverables.length > 0 && (
        <section>
          <h4 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-gray-500">
            Expected deliverables
          </h4>
          <ul className="flex flex-col gap-1 text-xs">
            {brief.expected_deliverables.map((d, idx) => (
              <li key={idx} className="flex items-center gap-2">
                <span className="font-mono">{d.filename}</span>
                <span className="rounded bg-gray-100 px-1.5 py-0.5 text-[10px] font-mono uppercase text-gray-700 dark:bg-gray-800 dark:text-gray-300">
                  {d.format}
                </span>
                {d.description && (
                  <span className="text-gray-500">— {d.description}</span>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

function BriefEditor({
  value,
  onChange,
  parseError,
}: {
  value: string;
  onChange: (v: string) => void;
  parseError: string | null;
}) {
  return (
    <div>
      <textarea
        value={value}
        onChange={(e) => onChange(e.target.value)}
        spellCheck={false}
        className="h-64 w-full resize-y rounded border border-gray-300 bg-gray-50 p-2 font-mono text-xs text-gray-900 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:border-gray-700 dark:bg-gray-900 dark:text-gray-100"
      />
      {parseError && (
        <p className="mt-1 text-xs text-red-700 dark:text-red-300">
          JSON parse error: {parseError}
        </p>
      )}
    </div>
  );
}

function formatTime(ts: number): string {
  try {
    return new Date(ts).toLocaleTimeString();
  } catch {
    return "";
  }
}
