"use client";

// Story 2.22: DeliverablesTab — fetches GET /v1/tasks/:id/deliverables,
// lists each row's filename + format + size, links into the existing
// workspace proxy (/v1/workspace/:session_id/<rendered_content_path>)
// for download. No content-route was added in 2.22 — the workspace
// proxy was built for exactly this. Empty state when the task has no
// deliverable yet (Phase 2 caps tasks at one).
//
// refs: /specs/phase-2/architecture.md §6
// refs: /specs/phase-2/stories/story-2.22.md

import { useEffect, useState } from "react";
import {
  getTaskDeliverables,
  type Deliverable,
  type TaskDeliverablesResponse,
} from "@/lib/api";

const API_BASE =
  typeof window === "undefined"
    ? ""
    : process.env.NEXT_PUBLIC_API_URL ?? `http://${window.location.hostname}:3000`;

type Props = {
  taskId: string | null;
};

export function DeliverablesTab({ taskId }: Props) {
  const [data, setData] = useState<TaskDeliverablesResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (taskId === null) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setData(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    getTaskDeliverables(taskId)
      .then((res) => {
        if (cancelled) return;
        setData(res);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [taskId]);

  if (taskId === null) {
    return (
      <p className="text-sm text-gray-500">
        Pick a task on the left to view its deliverables.
      </p>
    );
  }
  if (loading && data === null) {
    return <p className="text-sm text-gray-500">Loading deliverables…</p>;
  }
  if (error) {
    return (
      <p className="text-sm text-red-600">
        Failed to load deliverables: {error}
      </p>
    );
  }
  if (data === null || data.deliverables.length === 0) {
    return (
      <p className="text-sm text-gray-500">
        No deliverables yet for this task.
      </p>
    );
  }
  return (
    <ul className="flex flex-col divide-y divide-gray-200 dark:divide-gray-800">
      {data.deliverables.map((d) => (
        <DeliverableRow
          key={d.id}
          deliverable={d}
          sessionId={data.latest_session_id}
        />
      ))}
    </ul>
  );
}

function DeliverableRow({
  deliverable,
  sessionId,
}: {
  deliverable: Deliverable;
  sessionId: string | null;
}) {
  const filename = filenameOf(deliverable.rendered_content_path);
  const downloadUrl =
    sessionId === null
      ? null
      : `${API_BASE}/v1/workspace/${encodeURIComponent(sessionId)}/${encodePath(
          deliverable.rendered_content_path,
        )}`;
  return (
    <li className="flex flex-col gap-1 px-3 py-2 text-xs">
      <div className="flex items-center gap-2">
        <FormatChip format={deliverable.format} />
        <span className="flex-1 truncate font-medium text-gray-900 dark:text-gray-100">
          {filename}
        </span>
        <span className="text-gray-500">{formatBytes(deliverable.content_size)}</span>
      </div>
      <div className="flex items-center justify-between text-[11px] text-gray-500">
        <span>{new Date(deliverable.created_at / 1000).toLocaleString()}</span>
        {downloadUrl ? (
          <a
            href={downloadUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="text-blue-600 hover:underline"
          >
            Download
          </a>
        ) : (
          <span className="italic">no session — download unavailable</span>
        )}
      </div>
    </li>
  );
}

function FormatChip({ format }: { format: string }) {
  return (
    <span className="rounded bg-gray-200 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-gray-700 dark:bg-gray-800 dark:text-gray-200">
      {format}
    </span>
  );
}

function filenameOf(path: string): string {
  const idx = path.lastIndexOf("/");
  return idx === -1 ? path : path.slice(idx + 1);
}

// Encode each path segment so spaces and unicode survive the proxy
// without breaking the workspace route's `*sub_path` wildcard match.
function encodePath(path: string): string {
  return path
    .split("/")
    .filter((s) => s.length > 0)
    .map(encodeURIComponent)
    .join("/");
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}
