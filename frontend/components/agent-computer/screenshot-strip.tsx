"use client";

import { useMemo, useState } from "react";
import type { ServerEvent } from "@/lib/ws-types";
import { listDir } from "@/lib/workspace";
import { Lightbox } from "@/components/agent-computer/lightbox";

type Props = {
  sessionId: string;
  events: ServerEvent[];
};

type TrackCRef = {
  path: string;
  sha256: string;
  size: number;
  content_type: string;
};

type Shot =
  | { key: string; kind: "ok"; path: string }
  | { key: string; kind: "skipped"; reason: string }
  | { key: string; kind: "broken"; path: string };

const MAX_VISIBLE = 100;

function toWorkspaceUrl(sessionId: string, path: string): string {
  const clean = path.replace(/^\/+/, "");
  const base =
    typeof window === "undefined"
      ? ""
      : process.env.NEXT_PUBLIC_API_URL ?? `http://${window.location.hostname}:3000`;
  return `${base}/v1/workspace/${encodeURIComponent(sessionId)}/${encodeURI(clean)}`;
}

export function ScreenshotStrip({ sessionId, events }: Props) {
  const [broken, setBroken] = useState<Set<string>>(new Set());
  const [openKey, setOpenKey] = useState<string | null>(null);
  const [older, setOlder] = useState<Shot[]>([]);
  const [olderHidden, setOlderHidden] = useState(0);

  const { liveShots, hiddenInLive } = useMemo(() => {
    const rows: Shot[] = [];
    let hidden = 0;
    for (let i = 0; i < events.length; i++) {
      const ev = events[i];
      if (ev.session_id !== sessionId) continue;
      const p = ev.payload as {
        kind?: string;
        kind_tag?: string;
        call_id?: string;
        file_ref?: TrackCRef;
        reason?: string;
      };
      if (p.kind !== "Misc") continue;
      const tag = p.kind_tag ?? p.kind ?? "";
      if (tag === "browser_track_c") {
        const callId = p.call_id ?? `event-${ev.id}`;
        const path = p.file_ref?.path;
        if (!path) continue;
        rows.push({ key: `${ev.id}:${callId}`, kind: "ok", path });
      } else if (tag === "browser_track_c_skipped") {
        const callId = p.call_id ?? `event-${ev.id}`;
        rows.push({
          key: `${ev.id}:${callId}`,
          kind: "skipped",
          reason: p.reason ?? "unknown",
        });
      }
      if (rows.length > MAX_VISIBLE) {
        rows.shift();
        hidden += 1;
      }
    }
    return { liveShots: rows, hiddenInLive: hidden };
  }, [events, sessionId]);

  const effectiveOlderHidden = olderHidden > 0 ? olderHidden : hiddenInLive;
  const shots = older.concat(liveShots);

  const openShot = shots.find((s) => s.key === openKey && s.kind !== "skipped");
  const openSrc = openShot && "path" in openShot ? toWorkspaceUrl(sessionId, openShot.path) : null;

  const loadOlder = async () => {
    try {
      const listing = await listDir(sessionId, ".tracks");
      const names = listing.entries
        .filter((e) => e.type === "file" && e.name.endsWith(".png"))
        .map((e) => e.name)
        .sort();

      const known = new Set(
        shots
          .filter((s): s is Extract<Shot, { kind: "ok" | "broken" }> => s.kind !== "skipped")
          .map((s) => s.path.split("/").pop() ?? ""),
      );
      const fresh = names.filter((n) => !known.has(n)).slice(0, 50);
      if (fresh.length === 0) return;
      const preload: Shot[] = fresh.map((name, idx) => ({
        key: `older:${name}:${idx}`,
        kind: "ok",
        path: `/workspace/.tracks/${name}`,
      }));
      setOlder((prev) => preload.concat(prev));
      setOlderHidden((prev) => Math.max(0, prev - fresh.length));
    } catch {
      // Best-effort only.
    }
  };

  return (
    <div className="flex h-full flex-col gap-2">
      <div className="flex items-center justify-between text-[11px] text-gray-500">
        <span>Screenshots</span>
        {effectiveOlderHidden > 0 && (
          <button
            type="button"
            onClick={() => void loadOlder()}
            className="rounded border px-2 py-0.5 hover:bg-gray-50 dark:hover:bg-gray-800"
          >
            older screenshots hidden ({effectiveOlderHidden}) · load 50
          </button>
        )}
      </div>
      <div className="flex h-20 gap-1 overflow-x-auto rounded border p-1">
        {shots.map((shot) => {
          if (shot.kind === "skipped") {
            return (
              <div
                key={shot.key}
                className="flex min-w-24 items-center justify-center rounded border bg-gray-100 px-2 text-[10px] text-gray-500 dark:bg-gray-900"
                title={shot.reason}
              >
                skipped: {shot.reason}
              </div>
            );
          }

          const src = toWorkspaceUrl(sessionId, shot.path);
          const isBroken = broken.has(shot.key) || shot.kind === "broken";
          if (isBroken) {
            return (
              <div
                key={shot.key}
                className="flex h-full min-w-20 items-center justify-center rounded border bg-gray-100 text-lg text-gray-500 dark:bg-gray-900"
                title="image unavailable"
              >
                ⛔
              </div>
            );
          }

          return (
            <img
              key={shot.key}
              src={src}
              alt="browser screenshot"
              className="h-full min-w-20 cursor-pointer rounded border object-cover"
              onClick={() => setOpenKey(shot.key)}
              onError={() => {
                setBroken((prev) => {
                  const next = new Set(prev);
                  next.add(shot.key);
                  return next;
                });
              }}
            />
          );
        })}
      </div>
      {openSrc && (
        <Lightbox
          src={openSrc}
          alt="browser screenshot fullsize"
          onClose={() => setOpenKey(null)}
        />
      )}
    </div>
  );
}
