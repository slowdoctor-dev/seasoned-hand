import {
  AgentComputerPlaceholder,
  ChatPlaceholder,
  TaskListPlaceholder,
} from "@/components/panels";
import { ThreePanelLayout } from "@/components/three-panel-layout";

export default function Home() {
  return (
    <ThreePanelLayout
      left={<TaskListPlaceholder />}
      center={<ChatPlaceholder />}
      right={<AgentComputerPlaceholder />}
    />
  );
}
