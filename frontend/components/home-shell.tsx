"use client";

import { useState } from "react";
import { Chat } from "@/components/chat";
import { AgentComputerPlaceholder } from "@/components/panels";
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
      right={<AgentComputerPlaceholder />}
    />
  );
}
