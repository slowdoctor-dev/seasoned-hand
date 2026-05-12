"use client";

import { useState } from "react";
import { AgentComputer } from "@/components/agent-computer";
import { Chat } from "@/components/chat";
import { TaskList } from "@/components/task-list";
import { ThreePanelLayout } from "@/components/three-panel-layout";

export function HomeShell() {
  const [sessionId, setSessionId] = useState<string | null>(null);

  return (
    <ThreePanelLayout
      left={<TaskList activeSessionId={sessionId} onSelect={setSessionId} />}
      center={
        <Chat sessionId={sessionId} onSessionCreated={setSessionId} />
      }
      right={<AgentComputer sessionId={sessionId} />}
    />
  );
}
