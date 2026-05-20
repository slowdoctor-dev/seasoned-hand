"use client";

// Story 2.22: ProjectList — left-side panel above TaskList. Active
// project highlight + inline "Create new project" form + the synthetic
// `__archive__` row that surfaces Phase 0/1 legacy sessions (task_id
// IS NULL). The sentinel stays a frontend-only concern — there's no
// `__archive__` row in the projects table.
//
// refs: /specs/phase-2/architecture.md §6
// refs: /specs/phase-2/stories/story-2.22.md

import { useEffect, useState } from "react";
import { createProject, listProjects, type Project } from "@/lib/api";

export const ARCHIVE_PROJECT_ID = "__archive__";

type Props = {
  activeProjectId: string | null;
  onSelect: (id: string | null) => void;
};

export function ProjectList({ activeProjectId, onSelect }: Props) {
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [draftTitle, setDraftTitle] = useState("");
  const [draftDescription, setDraftDescription] = useState("");
  const [submitting, setSubmitting] = useState(false);
  // Bumped by the create flow + the refresh button to trigger a re-list.
  const [refreshTick, setRefreshTick] = useState(0);

  useEffect(() => {
    let cancelled = false;
    listProjects()
      .then((rows) => {
        if (cancelled) return;
        setProjects(rows);
        setError(null);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
        // Setting `projects` to an empty array drops the
        // simultaneously-rendered "Loading…" row so the user sees a
        // single error state instead of two stacked status messages.
        // The synthetic `Archive (legacy)` row below still renders from
        // its hardcoded entry.
        setProjects([]);
      });
    return () => {
      cancelled = true;
    };
  }, [refreshTick]);

  const onSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const title = draftTitle.trim();
    if (!title || submitting) return;
    setSubmitting(true);
    try {
      const created = await createProject(
        title,
        draftDescription.trim() || null,
      );
      setDraftTitle("");
      setDraftDescription("");
      setShowCreate(false);
      setRefreshTick((t) => t + 1);
      onSelect(created.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="flex flex-col border-b">
      <div className="flex items-center justify-between border-b px-4 py-3">
        <h2 className="font-semibold">Projects</h2>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setRefreshTick((t) => t + 1)}
            className="text-xs text-gray-500 hover:text-gray-900"
            aria-label="Refresh projects"
            title="Refresh"
          >
            ↻
          </button>
          <button
            type="button"
            onClick={() => setShowCreate((v) => !v)}
            className="rounded bg-blue-600 px-2 py-1 text-xs font-medium text-white hover:bg-blue-700"
          >
            {showCreate ? "Cancel" : "+ New"}
          </button>
        </div>
      </div>
      {showCreate && (
        <form onSubmit={onSubmit} className="border-b bg-gray-50 px-4 py-3 dark:bg-gray-900">
          <input
            type="text"
            placeholder="Project title"
            value={draftTitle}
            onChange={(e) => setDraftTitle(e.target.value)}
            className="mb-2 w-full rounded border border-gray-300 px-2 py-1 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:border-gray-700 dark:bg-black"
            autoFocus
          />
          <input
            type="text"
            placeholder="Description (optional)"
            value={draftDescription}
            onChange={(e) => setDraftDescription(e.target.value)}
            className="mb-2 w-full rounded border border-gray-300 px-2 py-1 text-xs focus:outline-none focus:ring-2 focus:ring-blue-500 dark:border-gray-700 dark:bg-black"
          />
          <button
            type="submit"
            disabled={submitting || draftTitle.trim() === ""}
            className="rounded bg-blue-600 px-3 py-1 text-xs font-medium text-white disabled:cursor-not-allowed disabled:bg-gray-400"
          >
            Create
          </button>
        </form>
      )}
      {error && (
        <p className="px-4 py-2 text-xs text-red-600">{error}</p>
      )}
      <ul className="max-h-[40vh] overflow-auto">
        {projects === null && (
          <li className="px-4 py-2 text-sm text-gray-500">Loading…</li>
        )}
        {projects !== null && projects.length === 0 && (
          <li className="px-4 py-2 text-sm text-gray-500">
            No projects yet. Click <em>+ New</em> to create one.
          </li>
        )}
        {projects?.map((p) => (
          <ProjectRow
            key={p.id}
            project={p}
            isActive={p.id === activeProjectId}
            onClick={() => onSelect(p.id)}
          />
        ))}
        <ArchiveRow
          isActive={activeProjectId === ARCHIVE_PROJECT_ID}
          onClick={() => onSelect(ARCHIVE_PROJECT_ID)}
        />
      </ul>
    </div>
  );
}

function ProjectRow({
  project,
  isActive,
  onClick,
}: {
  project: Project;
  isActive: boolean;
  onClick: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={onClick}
        className={`w-full border-b px-4 py-2 text-left text-sm hover:bg-gray-50 dark:hover:bg-gray-900 ${
          isActive
            ? "border-l-4 border-l-blue-500 bg-blue-50/40 dark:bg-blue-900/20"
            : ""
        }`}
      >
        <div className="truncate font-medium">{project.title}</div>
        {project.description && (
          <div className="mt-0.5 truncate text-xs text-gray-500">
            {project.description}
          </div>
        )}
        {project.status === "archived" && (
          <div className="mt-1 text-[10px] uppercase text-gray-400">
            archived
          </div>
        )}
      </button>
    </li>
  );
}

function ArchiveRow({
  isActive,
  onClick,
}: {
  isActive: boolean;
  onClick: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={onClick}
        className={`w-full border-b px-4 py-2 text-left text-sm italic text-gray-600 hover:bg-gray-50 dark:hover:bg-gray-900 ${
          isActive
            ? "border-l-4 border-l-blue-500 bg-blue-50/40 dark:bg-blue-900/20"
            : ""
        }`}
        title="Phase 0/1 legacy sessions (no project_id)"
      >
        <div className="truncate font-medium">Archive (legacy)</div>
        <div className="mt-0.5 truncate text-xs text-gray-500">
          Phase 0/1 sessions without a project
        </div>
      </button>
    </li>
  );
}
