# Story 0.26 — Monaco editor + file tree (read-only)

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 0.23 (tabs), backend `GET /v1/workspace/:session_id/*path` from architecture §4.1
> **Phase**: 0
> **Type**: frontend + backend (one tiny route)

## Goal

Editor tab: a file tree on the left, Monaco on the right, both
read-only views of the session's sandbox workspace. Path traversal is
blocked at the backend route.

## Acceptance criteria

### Backend
- [ ] `GET /v1/workspace/:session_id/*path` — when `path` ends with
      `/` or matches a directory, returns `{"type":"dir", "entries":
      [{name, type:"file"|"dir", size?}]}`; otherwise returns the file
      bytes with `Content-Type: text/plain; charset=utf-8` (Phase 0:
      treat all files as text; binary detection is Phase 1)
- [ ] Path traversal blocked: any segment `..` or absolute path
      returns 400
- [ ] If no sandbox for the session: 404
- [ ] Capped at 1 MB per file response (DEBT note)

### Frontend
- [ ] `frontend/package.json` adds `@monaco-editor/react` +
      `monaco-editor`
- [ ] `<EditorTab sessionId={id}/>` shows two columns: tree (left,
      ~30%), Monaco (right)
- [ ] Tree fetches `/v1/workspace/:id/` on mount, lazy-loads subdirs
      on expand
- [ ] Clicking a file fetches its content and loads it into Monaco
- [ ] Monaco `readOnly: true`, language detected by file extension
      (use `monaco.languages.getLanguages()` + extension map)
- [ ] `pnpm typecheck / lint / build` pass

## Non-goals

- Editing / saving (Phase 1)
- Binary files / images (Phase 1)
- Multi-file tabs (Phase 1)

## Files changed

- `crates/seasoned-hand-server/src/lib.rs` (workspace proxy route)
- `crates/seasoned-hand-server/tests/workspace.rs` (new — traversal block test)
- `frontend/package.json`
- `frontend/components/agent-computer/editor-tab.tsx` (new)
- `frontend/components/agent-computer/file-tree.tsx` (new)
- `specs/phase-0/DEBT.md` (1MB cap, binary handling deferred)

## Spec references

- `/specs/phase-0/architecture.md` §1 (Monaco), §4.1 (workspace proxy)
- `/specs/phase-0/architecture.md` §9 (path traversal protection)

## Commit message

```
feat(phase-0): story 0.26 - Monaco editor + file tree (read-only)

- backend: GET /v1/workspace/:session_id/*path proxies sandbox
  workspace; dir → JSON listing, file → text bytes; .. segments
  return 400; 1MB response cap; 404 if no sandbox
- frontend: @monaco-editor/react + @monaco/editor; EditorTab with
  lazy file tree + Monaco read-only viewer, language by extension
- pnpm typecheck/lint/build pass

Debt: 1MB file cap + text-only assumption (binary detection = Phase 1).

refs: /specs/phase-0/stories/story-0.26.md
```
