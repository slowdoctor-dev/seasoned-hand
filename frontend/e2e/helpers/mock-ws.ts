// Story 2.24: Playwright mock WebSocket helper. Intercepts the HomeShell's
// `ws://${hostname}:3000/ws` fallback via `page.routeWebSocket` (Playwright
// 1.48+) so smoke specs can script a server-side conversation without
// running the Rust control plane.
//
// The mock auto-acks every inbound client command (the BriefingCard
// confirm flow awaits its ack before flipping the resolution pill), and
// exposes:
//   - `script(events)` / `scriptOne(event)` to push canned ServerEvents
//     into the page once the socket is open
//   - `received` / `waitForCommand(predicate)` for assertions about what
//     the client emitted
//
// Bind a single instance per `test`, call `await mock.install(page)`
// before `page.goto("/")` so the routeWebSocket handler is registered
// when useAgentSocket's effect fires on mount.
//
// refs: /specs/phase-2/stories/story-2.24.md

import type { Page, WebSocketRoute } from "@playwright/test";
import type {
  ClientCommand,
  CommandPayload,
  ServerAck,
  ServerEvent,
} from "../../lib/ws-types";

export type MockWsOptions = {
  /**
   * Override the response for a specific cmd. Returning `null` keeps the
   * default auto-ack; returning an ack overrides it. The default is
   * `{ok: true, session_id?: deriveDefault}` — task_create gets a fresh
   * session_id so the chat UI can persist it.
   */
  ackResponder?: (cmd: CommandPayload, id: string) => ServerAck | null;
};

export type ReceivedCommand = {
  id: string;
  payload: CommandPayload;
};

type Resolver = {
  predicate: (cmd: ReceivedCommand) => boolean;
  resolve: (cmd: ReceivedCommand) => void;
};

export class MockAgentSocket {
  readonly received: ReceivedCommand[] = [];
  private route: WebSocketRoute | null = null;
  private resolvers: Resolver[] = [];
  private opened: Promise<void>;
  private markOpened!: () => void;

  constructor(private readonly options: MockWsOptions = {}) {
    this.opened = new Promise((res) => {
      this.markOpened = res;
    });
  }

  /**
   * Register the routeWebSocket handler on the page. Call BEFORE
   * `page.goto(...)` so useAgentSocket's mount-time WebSocket
   * construction is intercepted.
   */
  async install(page: Page): Promise<void> {
    // The HomeShell builds `ws://${window.location.hostname}:3000/ws`.
    // Match by suffix so it works against `localhost` or `127.0.0.1`.
    await page.routeWebSocket(/\/ws$/, (ws) => {
      this.route = ws;
      this.markOpened();
      ws.onMessage((raw) => this.handleClientMessage(raw));
      ws.onClose(() => {
        this.route = null;
      });
    });
  }

  /** Wait until the page has actually opened the mocked socket. */
  async waitOpen(): Promise<void> {
    await this.opened;
  }

  /** Push a single ServerEvent down to the page. */
  async scriptOne(event: ServerEvent): Promise<void> {
    await this.opened;
    if (!this.route) throw new Error("mock ws: route closed");
    this.route.send(JSON.stringify(event));
  }

  /** Push a batch of ServerEvents in order. */
  async script(events: ServerEvent[]): Promise<void> {
    for (const ev of events) await this.scriptOne(ev);
  }

  /**
   * Resolve when a command matching `predicate` is received. If a
   * matching command was already received it resolves synchronously on
   * the next microtask.
   */
  waitForCommand(
    predicate: (cmd: ReceivedCommand) => boolean,
    timeoutMs = 5_000,
  ): Promise<ReceivedCommand> {
    const existing = this.received.find(predicate);
    if (existing) return Promise.resolve(existing);
    return new Promise<ReceivedCommand>((resolve, reject) => {
      const timer = setTimeout(() => {
        const idx = this.resolvers.findIndex((r) => r.resolve === inner);
        if (idx >= 0) this.resolvers.splice(idx, 1);
        reject(new Error("mock ws: waitForCommand timeout"));
      }, timeoutMs);
      const inner = (cmd: ReceivedCommand) => {
        clearTimeout(timer);
        resolve(cmd);
      };
      this.resolvers.push({ predicate, resolve: inner });
    });
  }

  private handleClientMessage(raw: string | Buffer): void {
    const text = typeof raw === "string" ? raw : raw.toString("utf8");
    let parsed: ClientCommand;
    try {
      parsed = JSON.parse(text) as ClientCommand;
    } catch {
      return;
    }
    if (parsed.type !== "command") return;
    const entry: ReceivedCommand = { id: parsed.id, payload: parsed.payload };
    this.received.push(entry);
    for (let i = this.resolvers.length - 1; i >= 0; i--) {
      const r = this.resolvers[i];
      if (r.predicate(entry)) {
        this.resolvers.splice(i, 1);
        r.resolve(entry);
      }
    }
    const override = this.options.ackResponder?.(parsed.payload, parsed.id);
    const ack = override ?? defaultAck(parsed.payload, parsed.id);
    if (this.route) this.route.send(JSON.stringify(ack));
  }
}

function defaultAck(cmd: CommandPayload, id: string): ServerAck {
  const ack: ServerAck = { type: "ack", id, ref: id, ok: true };
  // task_create needs a session_id so the chat UI can latch it as
  // active. Real server generates a UUID; the mock fakes a stable one.
  if (cmd.cmd === "task_create") ack.session_id = "mock-session-1";
  return ack;
}

/** Build a Misc-tagged ServerEvent. Mirrors `build_payload` in ws.rs. */
export function miscEvent(args: {
  id: string;
  sessionId: string;
  ts?: number;
  kindTag: string;
  data?: Record<string, unknown>;
  extra?: Record<string, unknown>;
}): ServerEvent {
  const { id, sessionId, ts = 0, kindTag, data, extra } = args;
  return {
    type: "event",
    id,
    session_id: sessionId,
    ts,
    payload: {
      kind: "Misc",
      kind_tag: kindTag,
      ...(data !== undefined ? { data } : {}),
      ...(extra ?? {}),
    },
  };
}
