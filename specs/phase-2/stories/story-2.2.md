# Story 2.2 — V006 migration + ProjectStore + TaskStore

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 2.1
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.1, §3 V006

---

## Goal

Land the schema substrate Phase 2 sits on: a `projects` table, a `tasks`
table, and `sessions.task_id` FK. The two corresponding rusqlite-backed
stores expose CRUD + pagination. Multi-tenancy + skill/workflow slot
columns are nullable and populated only when later phases need them.

## Acceptance criteria

- [ ] `crates/seasoned-hand-core/migrations/V006__phase2_projects_tasks.sql`
      creates `projects` + `tasks` tables matching architecture §3 V006
      verbatim. `sessions.task_id` is added as a nullable FK + index.
      All new tables carry nullable `tenant_id`. `tasks` carries nullable
      `parent_task_id`, `schedule`, `skill_attached_event_id` slot
      reservation columns.
- [ ] `seasoned_hand_core::project::{Project, ProjectStore, NewProject}`
      exposes `insert / get / list / patch / set_status`. List is
      paginated by `created_at DESC` with `cursor` (created_at lower
      bound) + `limit` (default 50).
- [ ] `seasoned_hand_core::project::{Task, TaskStore, NewTask, TaskStatus}`
      exposes `insert / get / list_by_project / set_status /
      set_brief / set_failure / set_completed`. List is paginated +
      filterable by `status`.
- [ ] State transitions in `TaskStore::set_status` reject illegal moves
      per the state machine (drafted → briefed → confirmed → running ⇄
      paused → completed | failed | cancelled). Reject is a typed
      `TaskError::IllegalTransition`, not a panic.
- [ ] All SQL uses `rusqlite::params!` parameterized binding (no
      `format!` SQL).
- [ ] Unit tests: `project_store_crud`, `task_store_crud`,
      `task_state_machine_legal_transitions`,
      `task_state_machine_rejects_illegal_transitions`,
      `task_list_paginates_newest_first`,
      `project_list_filters_by_status`.

## Non-goals

- V007/V008/V009 migrations (story 2.3).
- HTTP routes (covered in stories 2.4 / 2.5 / 2.10).
- Briefing logic (story 2.7-2.8).
- Wiring stores into `AppState` (story 2.3 handles the consolidated
  wiring once all stores exist).

---

## Implementation steps

### 1. Migration

Copy the V006 SQL verbatim from `phase-2/architecture.md` §3. Place
in `crates/seasoned-hand-core/migrations/V006__phase2_projects_tasks.sql`.
Confirm `refinery` picks it up (existing migration runner).

### 2. Module

```
crates/seasoned-hand-core/src/project/
  mod.rs        ← re-exports
  project.rs    ← Project + ProjectStore + NewProject + ProjectError
  task.rs       ← Task + TaskStore + NewTask + TaskStatus + TaskError
  tests.rs
```

### 3. State machine

`TaskStatus` enum with `as_db_str` (mirror Phase 1 `VerdictKind`
pattern). Helper `fn legal_transitions(from: TaskStatus) -> &'static [TaskStatus]`.
`TaskStore::set_status` reads current status, validates, writes.

### 4. Pagination

Mirror Phase 1's `ListQuery { cursor: Option<i64>, limit: Option<usize> }`
+ `ListResponse<T> { rows, next_cursor }` pattern. Reuse the shared
`seasoned_hand_core::routes::RouteOutcome` type for the route-level
wrapper (used by 2.5 + 2.22; this story exports the store-level types
only).

### 5. Tests

In-memory DB fixture (`crate::db::open(":memory:")`). State-machine
test pins both legal AND illegal paths.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core project::
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/migrations/V006__phase2_projects_tasks.sql` (new)
- `crates/seasoned-hand-core/src/project/mod.rs` (new)
- `crates/seasoned-hand-core/src/project/project.rs` (new)
- `crates/seasoned-hand-core/src/project/task.rs` (new)
- `crates/seasoned-hand-core/src/project/tests.rs` (new)
- `crates/seasoned-hand-core/src/lib.rs` (modify — `pub mod project;`)

---

## Spec references

- `/specs/phase-2/architecture.md` §2.1 (Project / Task hierarchy),
  §3 (V006 migration)

---

## Commit message

```
feat(phase-2): story 2.2 - V006 migration + ProjectStore + TaskStore

- V006 creates projects + tasks tables (architecture §3 verbatim) +
  sessions.task_id FK. All new tables carry nullable tenant_id;
  tasks reserves parent_task_id / schedule /
  skill_attached_event_id slot columns for Phase 3-5.
- ProjectStore: insert / get / list (paginated by created_at DESC) /
  patch / set_status.
- TaskStore: insert / get / list_by_project (paginated, filterable by
  status) / set_status / set_brief / set_failure / set_completed.
  set_status enforces the drafted → briefed → confirmed → running ⇄
  paused → completed | failed | cancelled state machine; illegal
  transitions return TaskError::IllegalTransition.
- All SQL parameterized (rusqlite::params!).
- 6 unit tests.

refs: /specs/phase-2/stories/story-2.2.md
```

---

## Notes for next story (2.3)

V006 ships; sessions.task_id is nullable and Phase 0/1 rows stay
NULL (rendered under a synthetic "Phase 0/1 Archive" project by the
frontend later). 2.3 ships V007 + V008 + V009 in one consolidated
story.
