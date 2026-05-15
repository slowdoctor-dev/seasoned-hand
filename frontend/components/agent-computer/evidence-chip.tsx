"use client";

// Story 2.22: extracted from verifier-tab.tsx so the Decisions tab can
// reuse the chip for its evidence_event_ids array. Closes the 1.18-era
// "private helper" debt; behavior is unchanged.

import { useState } from "react";
import type { ServerEvent } from "@/lib/ws-types";

type Props = {
  eventId: number;
  eventIndex: Map<number, ServerEvent>;
};

export function EvidenceChip({ eventId, eventIndex }: Props) {
  const [open, setOpen] = useState(false);
  const ev = eventIndex.get(eventId);
  if (!ev) {
    return (
      <span
        className="cursor-default rounded bg-gray-200 px-2 py-0.5 text-[11px] text-gray-500 dark:bg-gray-800"
        title="Event is older than the currently loaded window"
      >
        #{eventId} (older than loaded window)
      </span>
    );
  }
  return (
    <span className="inline-flex flex-col items-start">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="rounded bg-blue-100 px-2 py-0.5 text-[11px] text-blue-800 hover:bg-blue-200 dark:bg-blue-900 dark:text-blue-100"
      >
        #{eventId}
      </button>
      {open && (
        <pre className="mt-1 max-h-40 max-w-md overflow-auto rounded bg-white p-2 text-[10px] dark:bg-black">
          {JSON.stringify(ev, null, 2)}
        </pre>
      )}
    </span>
  );
}
