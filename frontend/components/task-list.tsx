"use client";

import { useEffect, useMemo, useState } from "react";
import { listSessions, type SessionSummary } from "@/lib/api";
import { useAgentSocket } from "@/lib/ws";

type Props = {
  activeSessionId: string | null;
  onSelect: (id: string) => void;
};

const WS_URL =
  typeof window === "undefined"
    ? ""
    : process.env.NEXT_PUBLIC_WS_URL ?? `ws://${window.location.hostname}:3000/ws`;

export function TaskList({ activeSessionId, onSelect }: Props) {
  const [sessions, setSessions] = useState<SessionSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const { events } = useAgentSocket(WS_URL);

  const refetch = useMemo(
    () => async () => {
      try {
        const list = await listSessions();
        setSessions(list);
        setError(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [],
  );

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refetch();
  }, [refetch]);

  // Cheap Phase-0 invalidation: when a fresh session_id appears in WS
  // events that we don't know about, re-fetch the list.
  const known = useMemo(
    () => new Set(sessions?.map((s) => s.id) ?? []),
    [sessions],
  );
  useEffect(() => {
    if (sessions === null) return;
    for (const e of events) {
      if (!known.has(e.session_id)) {
        // Cheap Phase-0 invalidation: any unknown session_id in the WS
        // event stream triggers a list re-fetch. Replace with targeted
        // updates in Phase 1 (DEBT.md).
        // eslint-disable-next-line react-hooks/set-state-in-effect
        void refetch();
        return;
      }
    }
  }, [events, known, refetch, sessions]);

  if (error) {
    return (
      <aside className="h-full overflow-auto border-r p-4">
        <h2 className="mb-2 font-semibold">Tasks</h2>
        <p className="text-sm text-red-600">Failed to load: {error}</p>
      </aside>
    );
  }

  if (sessions === null) {
    return (
      <aside className="h-full overflow-auto border-r p-4">
        <h2 className="mb-2 font-semibold">Tasks</h2>
        <p className="text-sm text-gray-500">Loading…</p>
      </aside>
    );
  }

  if (sessions.length === 0) {
    return (
      <aside className="h-full overflow-auto border-r p-4">
        <h2 className="mb-2 font-semibold">Tasks</h2>
        <p className="text-sm text-gray-500">
          No tasks yet — type one in the center panel.
        </p>
      </aside>
    );
  }

  return (
    <aside className="h-full overflow-auto border-r">
      <h2 className="border-b px-4 py-3 font-semibold">Tasks</h2>
      <ul>
        {sessions.map((s) => {
          const isActive = s.id === activeSessionId;
          return (
            <li key={s.id}>
              <button
                type="button"
                onClick={() => onSelect(s.id)}
                className={`w-full border-b px-4 py-2 text-left text-sm hover:bg-gray-50 dark:hover:bg-gray-900 ${
                  isActive ? "border-l-4 border-l-blue-500 bg-blue-50/40 dark:bg-blue-900/20" : ""
                }`}
              >
                <div className="truncate font-medium">
                  {s.title ?? s.id.slice(0, 8)}
                </div>
                <div className="mt-1 flex items-center gap-2 text-xs text-gray-500">
                  <StateBadge state={s.state} />
                  <span>${(s.cost_cents / 100).toFixed(2)}</span>
                </div>
              </button>
            </li>
          );
        })}
      </ul>
    </aside>
  );
}

function StateBadge({ state }: { state: SessionSummary["state"] }) {
  const color: Record<SessionSummary["state"], string> = {
    IDLE: "bg-gray-200 text-gray-700",
    RUNNING: "bg-blue-200 text-blue-800",
    FINISHED: "bg-green-200 text-green-800",
    ERROR: "bg-red-200 text-red-800",
    SUSPENDED: "bg-yellow-200 text-yellow-800",
  };
  return (
    <span
      className={`rounded px-1.5 py-0.5 text-[10px] font-medium uppercase ${color[state]}`}
    >
      {state}
    </span>
  );
}
