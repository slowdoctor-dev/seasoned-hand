"use client";

import { useEffect, useState } from "react";

type Tab = "browser" | "terminal" | "editor" | "files";

type Props = {
  sessionId: string | null;
};

const TABS: { id: Tab; label: string; disabled?: boolean; note?: string }[] = [
  { id: "browser", label: "Browser" },
  { id: "terminal", label: "Terminal" },
  { id: "editor", label: "Editor" },
  { id: "files", label: "Files", disabled: true, note: "Phase 1" },
];

function storageKey(sessionId: string | null): string {
  return `sh.tab.${sessionId ?? "no-session"}`;
}

function initialTab(sessionId: string | null): Tab {
  if (typeof window === "undefined") return "browser";
  const saved = window.sessionStorage.getItem(storageKey(sessionId));
  if (saved === "browser" || saved === "terminal" || saved === "editor") {
    return saved;
  }
  return "browser";
}

export function AgentComputer({ sessionId }: Props) {
  const [active, setActive] = useState<Tab>(() => initialTab(sessionId));

  useEffect(() => {
    // Reset active tab when the session changes (read persisted choice
    // for the new session). Legit setState-in-effect: prop-driven reset.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setActive(initialTab(sessionId));
  }, [sessionId]);

  useEffect(() => {
    try {
      window.sessionStorage.setItem(storageKey(sessionId), active);
    } catch {
      // ignore storage failures
    }
  }, [active, sessionId]);

  return (
    <aside className="flex h-full flex-col border-l">
      <nav className="flex border-b">
        {TABS.map((t) => {
          const isActive = active === t.id;
          return (
            <button
              key={t.id}
              type="button"
              disabled={t.disabled}
              onClick={() => !t.disabled && setActive(t.id)}
              className={`relative flex-1 px-2 py-2 text-sm ${
                t.disabled
                  ? "cursor-not-allowed text-gray-400"
                  : isActive
                    ? "font-medium"
                    : "text-gray-600 hover:text-gray-900"
              }`}
            >
              {t.label}
              {t.note && (
                <span className="ml-1 text-[10px] uppercase text-gray-400">
                  {t.note}
                </span>
              )}
              {isActive && !t.disabled && (
                <span className="absolute inset-x-2 bottom-0 h-0.5 bg-blue-500" />
              )}
            </button>
          );
        })}
      </nav>
      <div className="flex-1 overflow-auto p-4 text-sm text-gray-500">
        {active === "browser" && (
          <Placeholder sessionId={sessionId} story="0.24" label="Browser" />
        )}
        {active === "terminal" && (
          <Placeholder sessionId={sessionId} story="0.25" label="Terminal" />
        )}
        {active === "editor" && (
          <Placeholder sessionId={sessionId} story="0.26" label="Editor" />
        )}
      </div>
    </aside>
  );
}

function Placeholder({
  sessionId,
  story,
  label,
}: {
  sessionId: string | null;
  story: string;
  label: string;
}) {
  return (
    <div>
      <p className="font-medium text-gray-700 dark:text-gray-300">{label}</p>
      <p className="mt-1 text-xs">
        Story {story} lands here. Session:{" "}
        <code className="rounded bg-gray-100 px-1 dark:bg-gray-800">
          {sessionId ?? "(none)"}
        </code>
      </p>
    </div>
  );
}
