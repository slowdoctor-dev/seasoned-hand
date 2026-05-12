"use client";

import { useEffect, useState, type ReactNode } from "react";
import { Group, type Layout, Panel, Separator } from "react-resizable-panels";

const STORAGE_KEY = "sh.panel-sizes.v1";
const PANEL_IDS = { left: "p-left", center: "p-center", right: "p-right" } as const;
const DEFAULT_LAYOUT: Layout = {
  [PANEL_IDS.left]: 20,
  [PANEL_IDS.center]: 50,
  [PANEL_IDS.right]: 30,
};

type Props = {
  left: ReactNode;
  center: ReactNode;
  right: ReactNode;
};

type Active = "left" | "center" | "right";

function loadLayout(): Layout {
  if (typeof window === "undefined") return DEFAULT_LAYOUT;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_LAYOUT;
    const parsed = JSON.parse(raw) as Layout;
    if (
      typeof parsed[PANEL_IDS.left] === "number" &&
      typeof parsed[PANEL_IDS.center] === "number" &&
      typeof parsed[PANEL_IDS.right] === "number"
    ) {
      return parsed;
    }
    return DEFAULT_LAYOUT;
  } catch {
    return DEFAULT_LAYOUT;
  }
}

function initialMobile(): boolean {
  if (typeof window === "undefined") return false;
  return window.matchMedia("(max-width: 767px)").matches;
}

export function ThreePanelLayout({ left, center, right }: Props) {
  // Lazy-init keeps the post-mount setState pattern out of useEffect
  // (linter rule react-hooks/set-state-in-effect). SSR sees the function
  // body skip via the typeof-window guard.
  const [mobile, setMobile] = useState<boolean>(initialMobile);
  const [active, setActive] = useState<Active>("center");
  const [layout] = useState<Layout>(loadLayout);

  useEffect(() => {
    const mq = window.matchMedia("(max-width: 767px)");
    const onChange = (e: MediaQueryListEvent) => setMobile(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  if (mobile) {
    return (
      <div className="flex h-screen flex-col">
        <nav className="flex border-b">
          {(["left", "center", "right"] as const).map((p) => (
            <button
              key={p}
              type="button"
              onClick={() => setActive(p)}
              className={`flex-1 px-2 py-2 text-sm ${
                active === p ? "bg-gray-200 font-medium" : ""
              }`}
            >
              {p === "left" ? "Tasks" : p === "center" ? "Chat" : "Agent"}
            </button>
          ))}
        </nav>
        <div className="flex-1 overflow-auto">
          {active === "left" && left}
          {active === "center" && center}
          {active === "right" && right}
        </div>
      </div>
    );
  }

  return (
    <Group
      orientation="horizontal"
      defaultLayout={layout}
      onLayoutChange={(next) => {
        try {
          window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
        } catch {
          // localStorage may be unavailable (private mode); silently drop.
        }
      }}
      className="h-screen"
    >
      <Panel id={PANEL_IDS.left} defaultSize={20} minSize={10}>
        {left}
      </Panel>
      <Separator
        className="w-1 cursor-col-resize transition-colors hover:bg-gray-300"
        aria-label="Resize left panel"
      />
      <Panel id={PANEL_IDS.center} defaultSize={50} minSize={30}>
        {center}
      </Panel>
      <Separator
        className="w-1 cursor-col-resize transition-colors hover:bg-gray-300"
        aria-label="Resize right panel"
      />
      <Panel id={PANEL_IDS.right} defaultSize={30} minSize={15}>
        {right}
      </Panel>
    </Group>
  );
}
