"use client";

// Story 2.22: TaskList dispatches by activeProjectId.
//
//   activeProjectId === null                → "Pick a project" empty state
//   activeProjectId === ARCHIVE_PROJECT_ID  → Phase 0/1 legacy GET /v1/sessions
//   activeProjectId === <uuid>              → GET /v1/projects/:id/tasks
//
// Project mode reports the selected task via onSelectTask. Archive mode
// reports the selected session via onSelectSession (legacy flow).

import { useEffect, useMemo, useState } from "react";
import { ARCHIVE_PROJECT_ID } from "@/components/project-list";
import {
  listSessions,
  listTasks,
  type SessionSummary,
  type Task,
} from "@/lib/api";
import { useAgentSocket } from "@/lib/ws";

type Props = {
  activeProjectId: string | null;
  activeTaskId: string | null;
  activeSessionId: string | null;
  onSelectTask: (id: string) => void;
  onSelectSession: (id: string) => void;
};

const WS_URL =
  typeof window === "undefined"
    ? ""
    : process.env.NEXT_PUBLIC_WS_URL ?? `ws://${window.location.hostname}:3000/ws`;

export function TaskList({
  activeProjectId,
  activeTaskId,
  activeSessionId,
  onSelectTask,
  onSelectSession,
}: Props) {
  if (activeProjectId === null) {
    return (
      <aside className="h-full overflow-auto p-4">
        <h2 className="mb-2 font-semibold">Tasks</h2>
        <p className="text-sm text-gray-500">
          Select a project above to see its tasks.
        </p>
      </aside>
    );
  }
  if (activeProjectId === ARCHIVE_PROJECT_ID) {
    return (
      <ArchiveSessions
        activeSessionId={activeSessionId}
        onSelect={onSelectSession}
      />
    );
  }
  return (
    <ProjectTasks
      projectId={activeProjectId}
      activeTaskId={activeTaskId}
      onSelect={onSelectTask}
    />
  );
}

function ProjectTasks({
  projectId,
  activeTaskId,
  onSelect,
}: {
  projectId: string;
  activeTaskId: string | null;
  onSelect: (id: string) => void;
}) {
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refetch = useMemo(
    () => async () => {
      try {
        const rows = await listTasks(projectId);
        setTasks(rows);
        setError(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [projectId],
  );

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setTasks(null);
    setError(null);
    void refetch();
  }, [refetch]);

  if (error) {
    return (
      <aside className="h-full overflow-auto p-4">
        <h2 className="mb-2 font-semibold">Tasks</h2>
        <p className="text-sm text-red-600">Failed to load: {error}</p>
      </aside>
    );
  }
  if (tasks === null) {
    return (
      <aside className="h-full overflow-auto p-4">
        <h2 className="mb-2 font-semibold">Tasks</h2>
        <p className="text-sm text-gray-500">Loading…</p>
      </aside>
    );
  }
  if (tasks.length === 0) {
    return (
      <aside className="h-full overflow-auto p-4">
        <h2 className="mb-2 font-semibold">Tasks</h2>
        <p className="text-sm text-gray-500">
          No tasks yet — type one in the center panel.
        </p>
      </aside>
    );
  }
  return (
    <aside className="h-full overflow-auto">
      <div className="flex items-center justify-between border-b px-4 py-3">
        <h2 className="font-semibold">Tasks</h2>
        <button
          type="button"
          onClick={() => void refetch()}
          className="text-xs text-gray-500 hover:text-gray-900"
          aria-label="Refresh tasks"
          title="Refresh"
        >
          ↻
        </button>
      </div>
      <ul>
        {tasks.map((t) => (
          <TaskRow
            key={t.id}
            task={t}
            isActive={t.id === activeTaskId}
            onClick={() => onSelect(t.id)}
          />
        ))}
      </ul>
    </aside>
  );
}

function ArchiveSessions({
  activeSessionId,
  onSelect,
}: {
  activeSessionId: string | null;
  onSelect: (id: string) => void;
}) {
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

  const known = useMemo(
    () => new Set(sessions?.map((s) => s.id) ?? []),
    [sessions],
  );
  useEffect(() => {
    if (sessions === null) return;
    for (const e of events) {
      if (!known.has(e.session_id)) {
        // eslint-disable-next-line react-hooks/set-state-in-effect
        void refetch();
        return;
      }
    }
  }, [events, known, refetch, sessions]);

  if (error) {
    return (
      <aside className="h-full overflow-auto p-4">
        <h2 className="mb-2 font-semibold">Archive (Phase 0/1 sessions)</h2>
        <p className="text-sm text-red-600">Failed to load: {error}</p>
      </aside>
    );
  }
  if (sessions === null) {
    return (
      <aside className="h-full overflow-auto p-4">
        <h2 className="mb-2 font-semibold">Archive (Phase 0/1 sessions)</h2>
        <p className="text-sm text-gray-500">Loading…</p>
      </aside>
    );
  }
  if (sessions.length === 0) {
    return (
      <aside className="h-full overflow-auto p-4">
        <h2 className="mb-2 font-semibold">Archive (Phase 0/1 sessions)</h2>
        <p className="text-sm text-gray-500">No legacy sessions.</p>
      </aside>
    );
  }
  return (
    <aside className="h-full overflow-auto">
      <h2 className="border-b px-4 py-3 font-semibold">
        Archive (Phase 0/1 sessions)
      </h2>
      <ul>
        {sessions.map((s) => {
          const isActive = s.id === activeSessionId;
          return (
            <li key={s.id}>
              <button
                type="button"
                onClick={() => onSelect(s.id)}
                className={`w-full border-b px-4 py-2 text-left text-sm hover:bg-gray-50 dark:hover:bg-gray-900 ${
                  isActive
                    ? "border-l-4 border-l-blue-500 bg-blue-50/40 dark:bg-blue-900/20"
                    : ""
                }`}
              >
                <div className="truncate font-medium">
                  {s.title ?? s.id.slice(0, 8)}
                </div>
                <div className="mt-1 flex items-center gap-2 text-xs text-gray-500">
                  <SessionStateBadge state={s.state} />
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

function TaskRow({
  task,
  isActive,
  onClick,
}: {
  task: Task;
  isActive: boolean;
  onClick: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={onClick}
        className={`w-full border-b px-4 py-2 text-left text-sm hover:bg-gray-50 dark:hover:bg-gray-900 ${
          isActive
            ? "border-l-4 border-l-blue-500 bg-blue-50/40 dark:bg-blue-900/20"
            : ""
        }`}
      >
        <div className="truncate font-medium">{task.title}</div>
        <div className="mt-1 flex items-center gap-2 text-xs text-gray-500">
          <TaskStatusBadge status={task.status} />
          {task.failure_reason && (
            <span className="truncate text-red-600">{task.failure_reason}</span>
          )}
        </div>
      </button>
    </li>
  );
}

function TaskStatusBadge({ status }: { status: Task["status"] }) {
  const color: Record<Task["status"], string> = {
    drafted: "bg-gray-200 text-gray-700",
    briefed: "bg-purple-200 text-purple-800",
    confirmed: "bg-indigo-200 text-indigo-800",
    running: "bg-blue-200 text-blue-800",
    paused: "bg-yellow-200 text-yellow-800",
    completed: "bg-green-200 text-green-800",
    failed: "bg-red-200 text-red-800",
    cancelled: "bg-gray-200 text-gray-600",
  };
  return (
    <span
      className={`rounded px-1.5 py-0.5 text-[10px] font-medium uppercase ${color[status]}`}
    >
      {status}
    </span>
  );
}

function SessionStateBadge({ state }: { state: SessionSummary["state"] }) {
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
