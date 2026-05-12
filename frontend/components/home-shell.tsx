"use client";

import { useState } from "react";
import { Chat } from "@/components/chat";
import {
  AgentComputerPlaceholder,
  TaskListPlaceholder,
} from "@/components/panels";
import { ThreePanelLayout } from "@/components/three-panel-layout";

export function HomeShell() {
  const [sessionId, setSessionId] = useState<string | null>(null);

  return (
    <ThreePanelLayout
      left={<TaskListPlaceholder />}
      center={
        <Chat sessionId={sessionId} onSessionCreated={setSessionId} />
      }
      right={<AgentComputerPlaceholder />}
    />
  );
}
