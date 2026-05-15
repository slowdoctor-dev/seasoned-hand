"use client";

import { useMemo, useState } from "react";
import { AgentComputer } from "@/components/agent-computer";
import { Chat } from "@/components/chat";
import { ProjectList } from "@/components/project-list";
import { TaskList } from "@/components/task-list";
import { ThreePanelLayout } from "@/components/three-panel-layout";
import { useAgentSocket } from "@/lib/ws";
import type { ServerEvent } from "@/lib/ws-types";

const WS_URL =
  typeof window === "undefined"
    ? ""
    : process.env.NEXT_PUBLIC_WS_URL ?? `ws://${window.location.hostname}:3000/ws`;

export function HomeShell() {
  // Story 2.22: HomeShell owns activeProjectId + activeTaskId + active
  // sessionId so TaskList / Chat / AgentComputer can coordinate. The
  // archive sentinel `__archive__` keeps archive-mode behavior pure
  // frontend; in project mode the user clicks tasks (taskId), in archive
  // mode the user clicks sessions (sessionId, legacy Phase 0/1 flow).
  const [activeProjectId, setActiveProjectId] = useState<string | null>(null);
  const [activeTaskId, setActiveTaskId] = useState<string | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const { events, send } = useAgentSocket(WS_URL);

  // Per-session event index used by the Verifier tab's evidence chips
  // and any future client-side lookup that wants O(1) resolution of an
  // event id to its body without an extra HTTP round-trip.
  const eventIndex = useMemo(() => {
    const map = new Map<number, ServerEvent>();
    for (const ev of events) {
      const id = Number.parseInt(ev.id, 10);
      if (!Number.isNaN(id)) map.set(id, ev);
    }
    return map;
  }, [events]);

  return (
    <ThreePanelLayout
      left={
        <aside className="flex h-full flex-col border-r">
          <ProjectList
            activeProjectId={activeProjectId}
            onSelect={(id) => {
              setActiveProjectId(id);
              // Switching project clears the active task selection but
              // leaves sessionId alone — chat's WS subscription survives
              // a project hop, which is what users expect mid-task.
              setActiveTaskId(null);
            }}
          />
          <div className="flex-1 overflow-hidden">
            <TaskList
              activeProjectId={activeProjectId}
              activeTaskId={activeTaskId}
              activeSessionId={sessionId}
              onSelectTask={setActiveTaskId}
              onSelectSession={setSessionId}
            />
          </div>
        </aside>
      }
      center={
        <Chat
          sessionId={sessionId}
          onSessionCreated={setSessionId}
          events={events}
          send={send}
        />
      }
      right={
        <AgentComputer
          sessionId={sessionId}
          taskId={activeTaskId}
          events={events}
          eventIndex={eventIndex}
        />
      }
    />
  );
}
