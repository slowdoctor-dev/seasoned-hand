"use client";

import { useMemo, useState } from "react";
import { AgentComputer } from "@/components/agent-computer";
import { Chat } from "@/components/chat";
import { TaskList } from "@/components/task-list";
import { ThreePanelLayout } from "@/components/three-panel-layout";
import { useAgentSocket } from "@/lib/ws";
import type { ServerEvent } from "@/lib/ws-types";

const WS_URL =
  typeof window === "undefined"
    ? ""
    : process.env.NEXT_PUBLIC_WS_URL ?? `ws://${window.location.hostname}:3000/ws`;

export function HomeShell() {
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
      left={<TaskList activeSessionId={sessionId} onSelect={setSessionId} />}
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
          events={events}
          eventIndex={eventIndex}
        />
      }
    />
  );
}
