"use client";

import { useEffect, useState } from "react";
import { BrowserTab } from "@/components/agent-computer/browser-tab";
import { DecisionsTab } from "@/components/agent-computer/decisions-tab";
import { DeliverablesTab } from "@/components/agent-computer/deliverables-tab";
import { EditorTab } from "@/components/agent-computer/editor-tab";
import { TerminalTab } from "@/components/agent-computer/terminal-tab";
import { VerifierTab } from "@/components/agent-computer/verifier-tab";
import type { ServerEvent } from "@/lib/ws-types";

type Tab =
  | "browser"
  | "terminal"
  | "editor"
  | "verifier"
  | "deliverables"
  | "decisions"
  | "files";

type Props = {
  sessionId: string | null;
  taskId: string | null;
  events: ServerEvent[];
  eventIndex: Map<number, ServerEvent>;
};

const TABS: { id: Tab; label: string; disabled?: boolean; note?: string }[] = [
  { id: "browser", label: "Browser" },
  { id: "terminal", label: "Terminal" },
  { id: "editor", label: "Editor" },
  { id: "verifier", label: "Verifier" },
  { id: "deliverables", label: "Deliverables" },
  { id: "decisions", label: "Decisions" },
  { id: "files", label: "Files", disabled: true, note: "Phase 1" },
];

function isPersistedTab(value: string | null): value is Tab {
  return (
    value === "browser" ||
    value === "terminal" ||
    value === "editor" ||
    value === "verifier" ||
    value === "deliverables" ||
    value === "decisions"
  );
}

function storageKey(sessionId: string | null): string {
  return `sh.tab.${sessionId ?? "no-session"}`;
}

function initialTab(sessionId: string | null): Tab {
  if (typeof window === "undefined") return "browser";
  const saved = window.sessionStorage.getItem(storageKey(sessionId));
  if (isPersistedTab(saved)) {
    return saved;
  }
  return "browser";
}

export function AgentComputer({ sessionId, taskId, events, eventIndex }: Props) {
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
    <aside className="flex h-full flex-col overflow-hidden border-l">
      {/* Tab strip can overflow the right panel's narrow default width
          (~430 px) — 7 labels at `text-sm` total ~500 px. Use a horizontal
          scroller so the parent `aside` keeps its own scrollbars hidden
          and the panel below stays full-bleed. `flex-shrink-0` on the
          tabs keeps each button at its natural width (no mid-label
          truncation). */}
      <nav className="flex flex-none overflow-x-auto whitespace-nowrap border-b [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {TABS.map((t) => {
          const isActive = active === t.id;
          return (
            <button
              key={t.id}
              type="button"
              disabled={t.disabled}
              onClick={() => !t.disabled && setActive(t.id)}
              className={`relative flex-none px-3 py-2 text-sm ${
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
          <BrowserTab
            sessionId={sessionId}
            events={events}
          />
        )}
        {active === "terminal" && <TerminalTab sessionId={sessionId} />}
        {active === "editor" && <EditorTab sessionId={sessionId} />}
        {active === "verifier" && (
          <VerifierTab
            sessionId={sessionId}
            events={events}
            eventIndex={eventIndex}
          />
        )}
        {active === "deliverables" && <DeliverablesTab taskId={taskId} />}
        {active === "decisions" && (
          <DecisionsTab
            sessionId={sessionId}
            events={events}
            eventIndex={eventIndex}
          />
        )}
      </div>
    </aside>
  );
}
