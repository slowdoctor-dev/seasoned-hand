"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { WS_AUTH_SUBPROTOCOL, authToken } from "./auth";
import type {
  ClientCommand,
  CommandPayload,
  ServerAck,
  ServerEnvelope,
  ServerEvent,
} from "./ws-types";

export type WsStatus = "connecting" | "open" | "closed" | "reconnecting";

type AckCallbacks = Map<string, (ack: ServerAck) => void>;

const EVENTS_CAP = 1000;
const BACKOFF_BASE_MS = 1000;
const BACKOFF_MAX_MS = 30_000;
const PONG_TIMEOUT_MS = 5_000;

function uuid(): string {
  // Avoid crypto.randomUUID requirement on older runtimes; small bias is fine
  // for client-side command ids.
  const hex = (n: number) => n.toString(16).padStart(2, "0");
  const bytes = new Uint8Array(16);
  if (typeof crypto !== "undefined" && "getRandomValues" in crypto) {
    crypto.getRandomValues(bytes);
  } else {
    for (let i = 0; i < 16; i++) bytes[i] = Math.floor(Math.random() * 256);
  }
  // RFC 4122 v4
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const b = Array.from(bytes, hex).join("");
  return `${b.slice(0, 8)}-${b.slice(8, 12)}-${b.slice(12, 16)}-${b.slice(
    16,
    20,
  )}-${b.slice(20, 32)}`;
}

export type UseAgentSocket = {
  status: WsStatus;
  events: ServerEvent[];
  lastEventId: string | null;
  send: (payload: CommandPayload) => Promise<ServerAck>;
};

export function useAgentSocket(url: string): UseAgentSocket {
  const [status, setStatus] = useState<WsStatus>("connecting");
  const [events, setEvents] = useState<ServerEvent[]>([]);
  const [lastEventId, setLastEventId] = useState<string | null>(null);

  const socketRef = useRef<WebSocket | null>(null);
  const ackCallbacksRef = useRef<AckCallbacks>(new Map());
  const subscribedSessionsRef = useRef<Map<string, number>>(new Map());
  const backoffRef = useRef(BACKOFF_BASE_MS);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pongTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const closedByUserRef = useRef(false);

  const clearReconnectTimer = () => {
    if (reconnectTimerRef.current !== null) {
      clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
  };

  const clearPongTimer = () => {
    if (pongTimerRef.current !== null) {
      clearTimeout(pongTimerRef.current);
      pongTimerRef.current = null;
    }
  };

  const scheduleReconnect = useCallback((connect: () => void) => {
    setStatus("reconnecting");
    clearReconnectTimer();
    const delay = backoffRef.current;
    backoffRef.current = Math.min(backoffRef.current * 2, BACKOFF_MAX_MS);
    reconnectTimerRef.current = setTimeout(connect, delay);
  }, []);

  const handleMessage = useCallback((env: ServerEnvelope, sock: WebSocket) => {
    switch (env.type) {
      case "event": {
        setLastEventId(env.id);
        setEvents((prev) => {
          const next = [...prev, env];
          if (next.length > EVENTS_CAP) next.splice(0, next.length - EVENTS_CAP);
          return next;
        });
        // Track max event id per session for reconnect replay.
        const sessionId = env.session_id;
        const idNum = Number.parseInt(env.id, 10);
        if (!Number.isNaN(idNum)) {
          const cur = subscribedSessionsRef.current.get(sessionId) ?? 0;
          if (idNum > cur) subscribedSessionsRef.current.set(sessionId, idNum);
        }
        break;
      }
      case "ack": {
        const cb = ackCallbacksRef.current.get(env.ref);
        if (cb) {
          ackCallbacksRef.current.delete(env.ref);
          cb(env);
        }
        break;
      }
      case "ping": {
        // Pong reply within 5s.
        sock.send(JSON.stringify({ type: "pong", ts: Date.now() }));
        clearPongTimer();
        pongTimerRef.current = setTimeout(() => {
          // If the server then misses our next ping window we'll see close.
        }, PONG_TIMEOUT_MS);
        break;
      }
      case "pong":
      case "error":
        break;
    }
  }, []);

  useEffect(() => {
    closedByUserRef.current = false;
    backoffRef.current = BACKOFF_BASE_MS;

    let cancelled = false;

    const connect = () => {
      if (cancelled) return;
      setStatus("connecting");
      // ADR-017: browser WS can't set headers, so carry the bearer token as the
      // second offered subprotocol alongside the sentinel the server echoes back.
      const sock = new WebSocket(url, [WS_AUTH_SUBPROTOCOL, authToken()]);
      socketRef.current = sock;

      sock.onopen = () => {
        backoffRef.current = BACKOFF_BASE_MS;
        setStatus("open");
        // Replay-resume: re-subscribe to known sessions with from_event_id.
        for (const [sessionId, fromId] of subscribedSessionsRef.current) {
          const id = uuid();
          const env: ClientCommand = {
            type: "command",
            id,
            session_id: sessionId,
            ts: Date.now(),
            payload: { cmd: "subscribe", session_id: sessionId, from_event_id: fromId },
          };
          sock.send(JSON.stringify(env));
        }
      };

      sock.onmessage = (msg) => {
        try {
          const parsed = JSON.parse(msg.data as string) as ServerEnvelope;
          handleMessage(parsed, sock);
        } catch {
          // ignore malformed server messages
        }
      };

      sock.onclose = () => {
        clearPongTimer();
        if (closedByUserRef.current || cancelled) {
          setStatus("closed");
          return;
        }
        scheduleReconnect(connect);
      };

      sock.onerror = () => {
        // onclose runs next; nothing to do here.
      };
    };

    connect();

    return () => {
      cancelled = true;
      closedByUserRef.current = true;
      clearReconnectTimer();
      clearPongTimer();
      const sock = socketRef.current;
      if (sock && sock.readyState === WebSocket.OPEN) sock.close();
    };
  }, [url, handleMessage, scheduleReconnect]);

  const send = useCallback<UseAgentSocket["send"]>(
    (payload) =>
      new Promise((resolve, reject) => {
        const sock = socketRef.current;
        if (!sock || sock.readyState !== WebSocket.OPEN) {
          reject(new Error("socket not open"));
          return;
        }
        const id = uuid();
        const env: ClientCommand = {
          type: "command",
          id,
          ts: Date.now(),
          ...(("session_id" in payload && typeof payload.session_id === "string")
            ? { session_id: payload.session_id }
            : {}),
          payload,
        };
        ackCallbacksRef.current.set(id, resolve);
        if (payload.cmd === "subscribe") {
          subscribedSessionsRef.current.set(
            payload.session_id,
            payload.from_event_id ?? 0,
          );
        }
        sock.send(JSON.stringify(env));
      }),
    [],
  );

  return { status, events, lastEventId, send };
}
