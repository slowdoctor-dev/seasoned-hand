"use client";

import { useEffect, useMemo, useState } from "react";
import type { ServerEvent } from "@/lib/ws-types";
import { readFile } from "@/lib/workspace";

type Props = {
  sessionId: string;
  events: ServerEvent[];
};

type EventPayloadBody =
  | { kind: "inline"; bytes: number[] }
  | {
      kind: "file_ref";
      path: string;
      content_type: string;
      sha256: string;
      size: number;
    };

function decodeInline(bytes: number[]): string {
  try {
    return new TextDecoder().decode(new Uint8Array(bytes));
  } catch {
    return "";
  }
}

export function DomTextPane({ sessionId, events }: Props) {
  const [text, setText] = useState<string>("");

  const latestRef = useMemo(() => {
    for (let i = events.length - 1; i >= 0; i--) {
      const ev = events[i];
      if (ev.session_id !== sessionId) continue;
      const payload = ev.payload as {
        kind?: string;
        kind_tag?: string;
        dom_text_ref?: EventPayloadBody;
      };
      if (payload.kind !== "Misc") continue;
      const tag = payload.kind_tag ?? payload.kind ?? "";
      if (tag !== "browser_track_b") continue;
      return payload.dom_text_ref;
    }
    return undefined;
  }, [events, sessionId]);

  useEffect(() => {
    let cancelled = false;

    const run = async () => {
      if (!latestRef) {
        setText("");
        return;
      }
      if (latestRef.kind === "inline") {
        if (!cancelled) setText(decodeInline(latestRef.bytes));
        return;
      }
      try {
        const next = await readFile(sessionId, latestRef.path);
        if (!cancelled) setText(next);
      } catch {
        if (!cancelled) setText("");
      }
    };

    void run();
    return () => {
      cancelled = true;
    };
  }, [latestRef, sessionId]);

  return (
    <pre className="h-full overflow-auto whitespace-pre-wrap font-mono text-xs text-gray-700 dark:text-gray-300">
      {text || "(no DOM snapshot yet)"}
    </pre>
  );
}
