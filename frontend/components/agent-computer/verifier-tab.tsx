"use client";

// Story 1.18: Verifier verdict pane. Shows the verifier_verdict rows
// produced by stories 1.9-1.12; newest-first; rows expand to evidence
// chips and the optional suggested_plan_update JSON. Hydrates on mount
// and on every new Misc{kind:"verifier_verdict"} arrival over WS.
//
// refs: /specs/phase-1/architecture.md §1, §12 q1
// refs: /specs/phase-1/stories/story-1.18.md

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { EvidenceChip } from "@/components/agent-computer/evidence-chip";
import {
  getVerification,
  listVerifications,
  type Verification,
} from "@/lib/api";
import type { ServerEvent } from "@/lib/ws-types";

type Props = {
  sessionId: string | null;
  events: ServerEvent[];
  eventIndex: Map<number, ServerEvent>;
};

function isVerdictEvent(ev: ServerEvent): boolean {
  const p = ev.payload as { kind?: string; kind_tag?: string } & Record<
    string,
    unknown
  >;
  if (p.kind !== "Misc") return false;
  const tag = (p.kind_tag ?? p.kind ?? "") as string;
  return tag === "verifier_verdict";
}

export function VerifierTab({ sessionId, events, eventIndex }: Props) {
  const [verdicts, setVerdicts] = useState<Verification[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Triggers a re-fetch; bumped on mount, sessionId change, and every
  // new verifier_verdict event over WS.
  const [refreshTick, setRefreshTick] = useState(0);
  const lastSeenEventIdRef = useRef<string | null>(null);

  // Watch the live WS event stream for new verifier_verdict Misc
  // events scoped to this session. Each new one bumps refreshTick so
  // the list re-fetches from the canonical HTTP endpoint (cheap and
  // simpler than reconciling client-side state with the DB row).
  useEffect(() => {
    if (sessionId === null) return;
    for (let i = events.length - 1; i >= 0; i--) {
      const ev = events[i];
      if (ev.session_id !== sessionId) continue;
      if (!isVerdictEvent(ev)) continue;
      if (ev.id === lastSeenEventIdRef.current) break;
      lastSeenEventIdRef.current = ev.id;
      setRefreshTick((t) => t + 1);
      break;
    }
  }, [events, sessionId]);

  useEffect(() => {
    if (sessionId === null) {
      // Legit setState-in-effect: prop-driven reset when the active
      // session is cleared. Matches the precedent in agent-computer.tsx.
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setVerdicts([]);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    listVerifications(sessionId, 50)
      .then((res) => {
        if (cancelled) return;
        setVerdicts(res.rows);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, refreshTick]);

  if (sessionId === null) {
    return (
      <p className="text-sm text-gray-500">
        Select a session to view verifier verdicts.
      </p>
    );
  }
  if (loading && verdicts.length === 0) {
    return <p className="text-sm text-gray-500">Loading verdicts…</p>;
  }
  if (error) {
    return (
      <p className="text-sm text-red-600">Failed to load verdicts: {error}</p>
    );
  }
  if (verdicts.length === 0) {
    return (
      <p className="text-sm text-gray-500">
        No verifier runs yet for this session.
      </p>
    );
  }
  return (
    <ul className="flex flex-col divide-y divide-gray-200 dark:divide-gray-800">
      {verdicts.map((v) => (
        <VerdictRow key={v.id} verdict={v} eventIndex={eventIndex} />
      ))}
    </ul>
  );
}

function VerdictRow({
  verdict,
  eventIndex,
}: {
  verdict: Verification;
  eventIndex: Map<number, ServerEvent>;
}) {
  const [expanded, setExpanded] = useState(false);
  const [detail, setDetail] = useState<Verification | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);

  const fetchDetail = useCallback(async () => {
    try {
      const fresh = await getVerification(verdict.id);
      setDetail(fresh);
    } catch (e: unknown) {
      setDetailError(e instanceof Error ? e.message : String(e));
    }
  }, [verdict.id]);

  const toggle = () => {
    setExpanded((prev) => {
      const next = !prev;
      if (next && detail === null && detailError === null) {
        void fetchDetail();
      }
      return next;
    });
  };

  const created = useMemo(() => formatTimestamp(verdict.created_at), [
    verdict.created_at,
  ]);
  const shown = detail ?? verdict;
  const reasonOneLine =
    shown.reason.length > 80 ? `${shown.reason.slice(0, 80)}…` : shown.reason;

  return (
    <li>
      <button
        type="button"
        onClick={toggle}
        className="flex w-full items-start gap-2 px-3 py-2 text-left text-xs hover:bg-gray-50 dark:hover:bg-gray-900"
      >
        <VerdictBadge verdict={shown.verdict} />
        <div className="flex-1">
          <div className="text-gray-900 dark:text-gray-100">
            {reasonOneLine}
          </div>
          <div className="mt-0.5 text-gray-500">
            {shown.trigger_kind} · {shown.model_id} · {created}
          </div>
        </div>
        <span
          aria-hidden
          className="text-gray-400"
        >
          {expanded ? "▾" : "▸"}
        </span>
      </button>
      {expanded && (
        <div className="space-y-2 bg-gray-50 px-3 py-2 text-xs dark:bg-gray-900">
          {detailError && (
            <p className="text-red-600">
              Failed to load detail: {detailError}
            </p>
          )}
          {shown.evidence_event_ids.length > 0 && (
            <div>
              <p className="mb-1 text-gray-500">Evidence:</p>
              <div className="flex flex-wrap gap-1">
                {shown.evidence_event_ids.map((id) => (
                  <EvidenceChip key={id} eventId={id} eventIndex={eventIndex} />
                ))}
              </div>
            </div>
          )}
          {shown.suggested_plan_update !== null &&
            shown.suggested_plan_update !== undefined && (
              <div>
                <p className="mb-1 text-gray-500">Suggested plan update:</p>
                <pre className="max-h-48 overflow-auto rounded bg-white p-2 text-[11px] dark:bg-black">
                  {JSON.stringify(shown.suggested_plan_update, null, 2)}
                </pre>
              </div>
            )}
        </div>
      )}
    </li>
  );
}

function VerdictBadge({ verdict }: { verdict: "pass" | "fail" }) {
  const cls =
    verdict === "pass"
      ? "bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-200"
      : "bg-red-100 text-red-700 dark:bg-red-900 dark:text-red-200";
  return (
    <span
      className={`rounded px-1.5 py-0.5 text-[10px] uppercase tracking-wide ${cls}`}
    >
      {verdict}
    </span>
  );
}

function formatTimestamp(unixSeconds: number): string {
  try {
    return new Date(unixSeconds * 1000).toLocaleString();
  } catch {
    return String(unixSeconds);
  }
}
