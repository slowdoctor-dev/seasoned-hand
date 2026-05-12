export function TaskListPlaceholder() {
  return (
    <aside className="h-full overflow-auto border-r p-4">
      <h2 className="mb-2 font-semibold">Tasks</h2>
      <p className="text-sm text-gray-500">Story 0.22 wires the real list.</p>
    </aside>
  );
}

export function ChatPlaceholder() {
  return (
    <section className="flex h-full flex-col p-4">
      <h2 className="mb-2 font-semibold">Chat</h2>
      <p className="text-sm text-gray-500">Story 0.21 wires the real chat.</p>
    </section>
  );
}

export function AgentComputerPlaceholder() {
  return (
    <aside className="h-full overflow-auto border-l p-4">
      <h2 className="mb-2 font-semibold">Agent Computer</h2>
      <p className="text-sm text-gray-500">
        Tabs (Browser / Terminal / Editor / Files) land in stories 0.23–0.26.
      </p>
    </aside>
  );
}
