"use client";

import "@xterm/xterm/css/xterm.css";
import { useCallback, useEffect, useRef, useState } from "react";
import { getSession } from "@/lib/api";

type Props = {
  sessionId: string | null;
};

type State =
  | { kind: "no_session" }
  | { kind: "no_sandbox" }
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; ttydUrl: string };

const BACKOFF_BASE_MS = 1000;
const BACKOFF_MAX_MS = 30_000;

export function TerminalTab({ sessionId }: Props) {
  const [state, setState] = useState<State>(
    sessionId === null ? { kind: "no_session" } : { kind: "loading" },
  );
  const containerRef = useRef<HTMLDivElement | null>(null);

  const fetchSandbox = useCallback(async () => {
    if (sessionId === null) {
      setState({ kind: "no_session" });
      return;
    }
    setState({ kind: "loading" });
    try {
      const detail = await getSession(sessionId);
      if (detail.sandbox) {
        setState({ kind: "ready", ttydUrl: detail.sandbox.ttyd_url });
      } else {
        setState({ kind: "no_sandbox" });
      }
    } catch (e) {
      setState({
        kind: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  }, [sessionId]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void fetchSandbox();
  }, [fetchSandbox]);

  useEffect(() => {
    if (state.kind !== "ready") return;
    const container = containerRef.current;
    if (!container) return;

    let disposed = false;
    let backoff = BACKOFF_BASE_MS;
    let term: import("@xterm/xterm").Terminal | null = null;
    let fit: import("@xterm/addon-fit").FitAddon | null = null;
    let ws: WebSocket | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let resizeObserver: ResizeObserver | null = null;

    void (async () => {
      // Lazy import to avoid SSR issues with the canvas-based xterm.
      const { Terminal } = await import("@xterm/xterm");
      const { FitAddon } = await import("@xterm/addon-fit");
      const { AttachAddon } = await import("@xterm/addon-attach");
      if (disposed) return;

      term = new Terminal({
        disableStdin: true,
        convertEol: true,
        fontFamily:
          "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
        fontSize: 12,
      });
      fit = new FitAddon();
      term.loadAddon(fit);
      term.open(container);
      fit.fit();

      resizeObserver = new ResizeObserver(() => {
        try {
          fit?.fit();
        } catch {
          // ignore during teardown
        }
      });
      resizeObserver.observe(container);

      const connect = () => {
        if (disposed) return;
        ws = new WebSocket(state.ttydUrl);
        const attach = new AttachAddon(ws);
        term?.loadAddon(attach);
        ws.onopen = () => {
          backoff = BACKOFF_BASE_MS;
        };
        ws.onclose = () => {
          if (disposed) return;
          reconnectTimer = setTimeout(connect, backoff);
          backoff = Math.min(backoff * 2, BACKOFF_MAX_MS);
        };
      };
      connect();
    })();

    return () => {
      disposed = true;
      if (reconnectTimer !== null) clearTimeout(reconnectTimer);
      resizeObserver?.disconnect();
      ws?.close();
      term?.dispose();
    };
  }, [state]);

  if (state.kind === "no_session") {
    return (
      <p className="text-sm text-gray-500">
        Select a task on the left to view its terminal.
      </p>
    );
  }
  if (state.kind === "loading") {
    return <p className="text-sm text-gray-500">Loading…</p>;
  }
  if (state.kind === "error") {
    return <p className="text-sm text-red-600">Failed to load: {state.message}</p>;
  }
  if (state.kind === "no_sandbox") {
    return (
      <p className="text-sm text-gray-500">
        Sandbox starts when the agent calls its first browser/shell/file tool.
      </p>
    );
  }
  return <div ref={containerRef} className="h-full w-full" />;
}
