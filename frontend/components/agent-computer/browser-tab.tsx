"use client";

import { useCallback, useEffect, useState } from "react";
import { getSession } from "@/lib/api";

type Props = {
  sessionId: string | null;
};

type Sandbox = { api_url: string; novnc_url: string; ttyd_url: string };

export function BrowserTab({ sessionId }: Props) {
  const [sandbox, setSandbox] = useState<Sandbox | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);

  const fetchDetail = useCallback(async () => {
    if (sessionId === null) {
      setSandbox(null);
      setError(null);
      return;
    }
    try {
      const detail = await getSession(sessionId);
      setSandbox(detail.sandbox);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [sessionId]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void fetchDetail();
  }, [fetchDetail]);

  if (sessionId === null) {
    return (
      <p className="text-sm text-gray-500">
        Select a task on the left to view its browser.
      </p>
    );
  }

  if (error) {
    return <p className="text-sm text-red-600">Failed to load: {error}</p>;
  }

  if (sandbox === null) {
    return (
      <p className="text-sm text-gray-500">
        Sandbox starts when the agent calls its first browser/shell/file tool.
      </p>
    );
  }

  return (
    <div className="flex h-full flex-col gap-2">
      <div className="flex items-center justify-between text-xs text-gray-500">
        <code className="truncate">
          {sandbox.novnc_url}
        </code>
        <button
          type="button"
          onClick={() => setReloadKey((n) => n + 1)}
          className="rounded border px-2 py-1 hover:bg-gray-50 dark:hover:bg-gray-800"
        >
          Reload
        </button>
      </div>
      <iframe
        key={reloadKey}
        src={sandbox.novnc_url}
        title="Sandbox browser (noVNC)"
        sandbox="allow-scripts allow-same-origin"
        className="h-full w-full border-0"
      />
    </div>
  );
}
