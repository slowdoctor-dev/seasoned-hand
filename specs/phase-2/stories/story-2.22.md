# Story 2.22 — Frontend: ProjectList + Deliverables + Decisions tabs

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 2.2, 2.3 (routes)
> **Phase**: 2
> **Type**: frontend
> **Reads first**: `/specs/phase-2/architecture.md` §6 "Frontend (Phase 0/1)"

---

## Goal

Add the three new frontend surfaces that don't depend on the
Briefing UI (which is its own story 2.23 because of the confirm
round-trip logic):

1. **ProjectList** panel (left side of HomeShell)
2. **Deliverables** tab in AgentComputer
3. **Decisions** tab in AgentComputer

## Acceptance criteria

- [ ] `frontend/components/project-list.tsx` (new) — vertical list of
      projects with active highlight + "Create new" button.
- [ ] `frontend/components/home-shell.tsx` (modify) — gains a
      `ProjectList` above the existing `TaskList` in the left panel.
      Active `project_id` lifted into `HomeShell` state; passed down
      to `TaskList` (renamed in concept to `TaskListInProject`) +
      `Chat` + `AgentComputer`.
- [ ] `frontend/components/task-list.tsx` (modify) — accepts
      `activeProjectId` prop; calls `GET /v1/projects/:id/tasks`
      instead of the legacy `GET /v1/sessions`. When `activeProjectId
      === null`, renders the Phase 0/1 archive view (Phase 0/1
      sessions where `task_id IS NULL`, exposed via a synthetic
      `__archive__` project_id).
- [ ] `frontend/components/agent-computer/deliverables-tab.tsx`
      (new) — lists `GET /v1/tasks/:id/deliverables`. Each row:
      filename + format chip + size + download button (links to
      `/v1/tasks/:id/deliverables/:did/content`). Empty state:
      "No deliverables yet for this task."
- [ ] `frontend/components/agent-computer/decisions-tab.tsx` (new)
      — subscribes to the lifted-from-Phase-1.18 WS event stream;
      filters `payload.kind === "Misc" && payload.kind_tag ===
      "decision"`. Each row: source / reason (truncated 120 chars) /
      evidence chip count. Click row → expand to show all evidence
      chips (reuse `EvidenceChip` from story 1.18 VerifierTab).
- [ ] `agent-computer.tsx` (modify) — register two new tabs:
      `"deliverables"` + `"decisions"`. Existing tabs (`browser`,
      `terminal`, `editor`, `verifier`, `files`) unchanged.
- [ ] `frontend/lib/api.ts` (modify) — add `Project`, `Task`,
      `Deliverable` types + `listProjects`, `getProject`,
      `createProject`, `listTasks`, `getTask`,
      `getTaskDeliverables` functions. Mirror the existing
      `Verification` pattern.
- [ ] TypeScript types match the Rust serde-derived JSON exactly
      (manually mirrored as in Phase 0/1 pattern; ts-rs codegen
      remains deferred).
- [ ] `pnpm typecheck` + `pnpm lint` + `pnpm build` all clean.

## Non-goals

- Briefing card (story 2.23)
- Playwright coverage (story 2.24)
- 3-track BrowserTab integration with Deliverables (BrowserTab from
  story 1.19 stays unchanged; Deliverables is a separate tab)
- Rich rendering of .docx / .pptx in-browser (download-only for Phase
  2; in-browser preview is Phase 4 if requested)

---

## Implementation steps

### 1. ProjectList component

```tsx
type Project = {
  id: string;
  title: string;
  description: string | null;
  status: "active" | "archived";
  task_counts: { running: number; completed: number; failed: number };
};

export function ProjectList({
  activeProjectId,
  onSelect,
  onCreate,
}: Props) {
  const [projects, setProjects] = useState<Project[]>([]);
  // useEffect: fetch GET /v1/projects on mount + on refresh tick
  // Render: active highlight + create button + project list
}
```

### 2. HomeShell refactor

Lift `activeProjectId` into HomeShell state. Pass down to
`TaskListInProject` (renamed `TaskList`) + `Chat` + `AgentComputer`.

### 3. TaskList refactor

Switch HTTP target to `GET /v1/projects/:id/tasks`. Special
`__archive__` project shows Phase 0/1 legacy sessions
(`GET /v1/sessions` with a filter; the route returns sessions where
`task_id IS NULL`).

### 4. DeliverablesTab + DecisionsTab

Same pattern as Phase 1's `VerifierTab` (story 1.18): WS event
subscription + initial HTTP fetch + lazy detail expansion. Reuse
`EvidenceChip` for Decisions.

### 5. AgentComputer tab registration

`TABS` const gains:
```ts
{ id: "deliverables", label: "Deliverables" },
{ id: "decisions", label: "Decisions" },
```

### 6. Manual smoke

After `pnpm dev`:
- Open `/`, click "Create new project", create a project
- Submit a task via Chat (legacy path) — appears under the active project
- Wait for task to produce a Deliverable (manual via test task) — appears in Deliverables tab
- Decisions tab shows verifier decisions from Phase 1 paths + Initializer briefing decisions

---

## Verification

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend lint
pnpm --dir frontend build
./scripts/spec-check.sh
```

---

## Files changed

- `frontend/components/project-list.tsx` (new)
- `frontend/components/home-shell.tsx` (modify — lift state)
- `frontend/components/task-list.tsx` (modify — accept activeProjectId)
- `frontend/components/agent-computer.tsx` (modify — register 2 tabs)
- `frontend/components/agent-computer/deliverables-tab.tsx` (new)
- `frontend/components/agent-computer/decisions-tab.tsx` (new)
- `frontend/lib/api.ts` (modify — Project/Task/Deliverable types + fns)

---

## Spec references

- `/specs/phase-2/architecture.md` §6 (Frontend changes)

---

## Commit message

```
feat(phase-2): story 2.22 - Frontend ProjectList + Deliverables + Decisions tabs

- ProjectList left-side panel above TaskList. Active project highlight
  + "Create new" button. GET /v1/projects.
- TaskList refactored to GET /v1/projects/:id/tasks. Synthetic
  __archive__ project surfaces Phase 0/1 task_id IS NULL legacy
  sessions.
- AgentComputer gains Deliverables + Decisions tabs:
  - Deliverables: GET /v1/tasks/:id/deliverables + download link to
    /v1/tasks/:id/deliverables/:did/content.
  - Decisions: WS event filter for Misc{kind:"decision"}. Reuses
    EvidenceChip from story 1.18.
- lib/api.ts: Project, Task, Deliverable types + accessors.
- pnpm typecheck + lint + build all clean.

refs: /specs/phase-2/stories/story-2.22.md
```

---

## Notes for next story (2.23)

ProjectList + Deliverables + Decisions are in. 2.23 ships the
Briefing card — the trickier UI because it intercepts a specific
event kind and has confirm/edit/cancel round-trip back through WS.
