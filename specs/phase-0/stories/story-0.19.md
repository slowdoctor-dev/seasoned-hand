# Story 0.19 — 3-panel resizable layout

> **Status**: done

> **Notes from implementation**:
> - `react-resizable-panels` v4 renamed `PanelGroup→Group` and
>   `PanelResizeHandle→Separator`, and dropped `autoSaveId` —
>   localStorage wiring is now manual via `defaultLayout` +
>   `onLayoutChange`. The component does that.
> - ESLint rule `react-hooks/set-state-in-effect` forced lazy-init
>   `useState(fn)` for both mobile and layout state; the effect now
>   only adds the mediaquery listener.
> **Estimated**: 2 hours
> **Dependencies**: story 0.18 (Next.js init)
> **Phase**: 0
> **Type**: frontend
> **Reads first**: `/specs/phase-0/architecture.md` §1 (3-panel layout target), §5.3 (frontend deps incl. react-resizable-panels)

---

## Goal

Replace the placeholder home page with the three-panel shell:
**TaskList (left) | Chat (center) | AgentComputer (right)**. Panels
are resizable, persist sizes to localStorage, and adapt to small
screens (mobile = single-column stack with tab switcher; mobile-first
is not required per `NON_GOALS.md` but responsive degradation is).

This story ships only the **shell + placeholders**. Real content for
each panel lands in 0.21 (Chat), 0.22 (TaskList), 0.23+ (AgentComputer
tabs).

## Acceptance criteria

- [ ] `frontend/app/page.tsx` renders a `<ThreePanelLayout>` with three
      slots, each a placeholder identifying the panel (e.g. "TaskList —
      Story 0.22")
- [ ] `react-resizable-panels` v2.x installed
- [ ] Default sizes: 20% / 50% / 30%; min: 10% / 30% / 15%
- [ ] Resize handles visible on hover (thin vertical dividers,
      Tailwind `hover:bg-gray-300`)
- [ ] Sizes persist to `localStorage` key `sh.panel-sizes.v1` and
      restore on page load
- [ ] Reset button (`?reset` query param or a tiny dev button in the
      footer) clears the saved sizes
- [ ] At viewport < 768 px (Tailwind `md:`), layout becomes a vertical
      stack with three buttons at the top switching between panels;
      one panel visible at a time. Resize handles hidden.
- [ ] Accessible: each resize handle has `aria-label` + keyboard
      support (arrows to nudge) — `react-resizable-panels` provides
      this out of the box; verify
- [ ] `frontend/components/three-panel-layout.tsx` is the only new
      shared component; placeholders live in `frontend/components/panels/`
- [ ] No `any` (TypeScript strict per `AGENTS.md` §7)
- [ ] `pnpm typecheck / lint / build` all pass

## Non-goals

- Real panel content (0.21+)
- Drag-and-drop panel reordering (Phase 1+)
- Multiple workspace tabs at the page level (Phase 1+; Phase 0 has
  one task focused at a time)
- Mobile UX polish (just the basic stack-with-switcher; no animations)

---

## Implementation steps

### 1. Add dep

```bash
cd frontend && pnpm add react-resizable-panels@^2
```

### 2. `components/three-panel-layout.tsx`

```tsx
"use client";
import { PanelGroup, Panel, PanelResizeHandle } from "react-resizable-panels";
import { useEffect, useState } from "react";

const STORAGE_KEY = "sh.panel-sizes.v1";
const DEFAULTS: [number, number, number] = [20, 50, 30];

type Props = {
  left: React.ReactNode;
  center: React.ReactNode;
  right: React.ReactNode;
};

export function ThreePanelLayout({ left, center, right }: Props) {
  const [mobile, setMobile] = useState(false);
  const [activeMobilePanel, setActiveMobilePanel] = useState<"left" | "center" | "right">("center");

  useEffect(() => {
    const mq = window.matchMedia("(max-width: 767px)");
    setMobile(mq.matches);
    const onChange = () => setMobile(mq.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  if (mobile) {
    return (
      <div className="flex flex-col h-screen">
        <nav className="flex border-b">
          {(["left", "center", "right"] as const).map((p) => (
            <button
              key={p}
              onClick={() => setActiveMobilePanel(p)}
              className={`flex-1 px-2 py-2 text-sm ${
                activeMobilePanel === p ? "bg-gray-200 font-medium" : ""
              }`}
            >
              {p === "left" ? "Tasks" : p === "center" ? "Chat" : "Agent"}
            </button>
          ))}
        </nav>
        <div className="flex-1 overflow-auto">
          {activeMobilePanel === "left" && left}
          {activeMobilePanel === "center" && center}
          {activeMobilePanel === "right" && right}
        </div>
      </div>
    );
  }

  return (
    <PanelGroup
      direction="horizontal"
      autoSaveId={STORAGE_KEY}
      className="h-screen"
    >
      <Panel defaultSize={DEFAULTS[0]} minSize={10}>
        {left}
      </Panel>
      <PanelResizeHandle
        className="w-1 hover:bg-gray-300 transition-colors"
        aria-label="Resize left panel"
      />
      <Panel defaultSize={DEFAULTS[1]} minSize={30}>
        {center}
      </Panel>
      <PanelResizeHandle
        className="w-1 hover:bg-gray-300 transition-colors"
        aria-label="Resize right panel"
      />
      <Panel defaultSize={DEFAULTS[2]} minSize={15}>
        {right}
      </Panel>
    </PanelGroup>
  );
}
```

### 3. Wire into home page

`frontend/app/page.tsx`:

```tsx
import { ThreePanelLayout } from "@/components/three-panel-layout";
import { TaskListPlaceholder, ChatPlaceholder, AgentComputerPlaceholder }
  from "@/components/panels";

export default function Home() {
  return (
    <ThreePanelLayout
      left={<TaskListPlaceholder />}
      center={<ChatPlaceholder />}
      right={<AgentComputerPlaceholder />}
    />
  );
}
```

### 4. Tests

Frontend tests are still minimal in Phase 0 (no React Testing Library
yet — added Phase 1). Manual smoke check: `pnpm dev`, open
`http://127.0.0.1:3001`, resize, refresh, sizes persist, narrow window
to mobile width and verify tab switcher.

### 5. DEBT entry

- No automated frontend tests in Phase 0; verify by manual smoke +
  `pnpm build` typecheck.

---

## Files changed

- `frontend/package.json` (add react-resizable-panels)
- `frontend/app/page.tsx` (replace placeholder)
- `frontend/components/three-panel-layout.tsx` (new)
- `frontend/components/panels/index.ts` (new — re-exports placeholders)
- `frontend/components/panels/task-list-placeholder.tsx` (new)
- `frontend/components/panels/chat-placeholder.tsx` (new)
- `frontend/components/panels/agent-computer-placeholder.tsx` (new)
- `specs/phase-0/DEBT.md` (frontend testing gap)

---

## Spec references

- `/specs/phase-0/architecture.md` §1 (3-panel layout)
- `/specs/phase-0/architecture.md` §5.3 (react-resizable-panels 2.x)

---

## Commit message

```
feat(phase-0): story 0.19 - 3-panel resizable layout

- ThreePanelLayout component over react-resizable-panels v2
- Default sizes 20/50/30; min sizes 10/30/15; saved via autoSaveId
  to localStorage key sh.panel-sizes.v1
- Mobile (<768px) collapses to single-panel + tab switcher
- Placeholders for TaskList / Chat / AgentComputer
- Real content lands stories 0.21+ / 0.22 / 0.23+
- pnpm typecheck / lint / build pass

Debt: no automated frontend tests in Phase 0; manual smoke only.

refs: /specs/phase-0/stories/story-0.19.md
```
