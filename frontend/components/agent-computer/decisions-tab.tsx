"use client";

// Story 2.22: DecisionsTab — filters the live WS event stream for
// Misc decision events emitted by Initializer / Verifier / Checkpoint
// Manager (architecture §2.5). Mirrors the verifier-tab tolerance for
// the Misc payload shape (kind_tag vs kind) — see verifier-tab.tsx for
// the precedent; the underlying core builder (provenance/builder.rs)
// checks `e.data.kind == "decision"`, so the same dual-field probe
// keeps both forms supported.
//
// refs: /specs/phase-2/architecture.md §2.5, §6
// refs: /specs/phase-2/stories/story-2.22.md

import { useMemo, useState } from "react";
import { EvidenceChip } from "@/components/agent-computer/evidence-chip";
import type { ServerEvent } from "@/lib/ws-types";

type Props = {
  sessionId: string | null;
  events: ServerEvent[];
  eventIndex: Map<number, ServerEvent>;
};

type DecisionRow = {
  eventId: number;
  ts: number;
  source: string;
  reason: string;
  evidenceEventIds: number[];
};

export function DecisionsTab({ sessionId, events, eventIndex }: Props) {
  const decisions = useMemo<DecisionRow[]>(() => {
    if (sessionId === null) return [];
    const rows: DecisionRow[] = [];
    for (const ev of events) {
      if (ev.session_id !== sessionId) continue;
      if (!isDecisionEvent(ev)) continue;
      const id = Number.parseInt(ev.id, 10);
      if (Number.isNaN(id)) continue;
      const p = ev.payload as Record<string, unknown>;
      rows.push({
        eventId: id,
        ts: ev.ts,
        source: pickString(p, "source") ?? "unknown",
        reason: pickString(p, "reason") ?? "",
        evidenceEventIds: pickEvidence(p),
      });
    }
    // Newest-first.
    rows.reverse();
    return rows;
  }, [events, sessionId]);

  if (sessionId === null) {
    return (
      <p className="text-sm text-gray-500">
        Select a session to view recorded decisions.
      </p>
    );
  }
  if (decisions.length === 0) {
    return (
      <p className="text-sm text-gray-500">
        No decisions recorded yet for this session.
      </p>
    );
  }
  return (
    <ul className="flex flex-col divide-y divide-gray-200 dark:divide-gray-800">
      {decisions.map((d) => (
        <DecisionRowView key={d.eventId} row={d} eventIndex={eventIndex} />
      ))}
    </ul>
  );
}

function DecisionRowView({
  row,
  eventIndex,
}: {
  row: DecisionRow;
  eventIndex: Map<number, ServerEvent>;
}) {
  const [expanded, setExpanded] = useState(false);
  const reasonOneLine =
    row.reason.length > 120 ? `${row.reason.slice(0, 120)}…` : row.reason;
  const created = useMemo(() => {
    try {
      return new Date(row.ts * 1000).toLocaleString();
    } catch {
      return String(row.ts);
    }
  }, [row.ts]);
  return (
    <li>
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="flex w-full items-start gap-2 px-3 py-2 text-left text-xs hover:bg-gray-50 dark:hover:bg-gray-900"
      >
        <span className="rounded bg-purple-100 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-purple-700 dark:bg-purple-900 dark:text-purple-200">
          {row.source}
        </span>
        <div className="flex-1">
          <div className="text-gray-900 dark:text-gray-100">
            {reasonOneLine || <em className="text-gray-500">(no reason)</em>}
          </div>
          <div className="mt-0.5 text-gray-500">
            #{row.eventId} · {row.evidenceEventIds.length} evidence · {created}
          </div>
        </div>
        <span aria-hidden className="text-gray-400">
          {expanded ? "▾" : "▸"}
        </span>
      </button>
      {expanded && (
        <div className="space-y-2 bg-gray-50 px-3 py-2 text-xs dark:bg-gray-900">
          {row.reason.length > 120 && (
            <p className="whitespace-pre-wrap text-gray-700 dark:text-gray-300">
              {row.reason}
            </p>
          )}
          {row.evidenceEventIds.length > 0 && (
            <div>
              <p className="mb-1 text-gray-500">Evidence:</p>
              <div className="flex flex-wrap gap-1">
                {row.evidenceEventIds.map((id) => (
                  <EvidenceChip key={id} eventId={id} eventIndex={eventIndex} />
                ))}
              </div>
            </div>
          )}
        </div>
      )}
    </li>
  );
}

function isDecisionEvent(ev: ServerEvent): boolean {
  const p = ev.payload as { kind?: string; kind_tag?: string };
  if (p.kind !== "Misc") return false;
  const tag = p.kind_tag ?? p.kind ?? "";
  return tag === "decision";
}

function pickString(payload: Record<string, unknown>, key: string): string | null {
  const v = payload[key];
  return typeof v === "string" ? v : null;
}

function pickEvidence(payload: Record<string, unknown>): number[] {
  const raw = payload["evidence_event_ids"] ?? payload["evidence"];
  if (!Array.isArray(raw)) return [];
  const ids: number[] = [];
  for (const v of raw) {
    if (typeof v === "number" && Number.isFinite(v)) ids.push(v);
    else if (typeof v === "string") {
      const n = Number.parseInt(v, 10);
      if (!Number.isNaN(n)) ids.push(n);
    }
  }
  return ids;
}
