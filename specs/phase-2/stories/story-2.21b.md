# Story 2.21b — `seasoned-hand` CLI binary (intake + brief + inbox + init/server)

> **Status**: ready (depends on 2.21a)
> **Estimated**: 2-3 hours
> **Dependencies**: 2.21a (project / task surface + CliChannel registration)
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.10 (CLI surface),
> `/specs/phase-2/stories/story-2.21.md` (parent), `/specs/phase-2/stories/story-2.21b.md` (this)

---

## Goal

Finish the OS-layer CLI surface deferred from 2.21a. The blocking
`task new`, briefing-card flow, inbox listing, channel test/logs
plumbing, and `init` / `server` bootstrap subcommands all land here so
operators have a complete CLI to drive Phase 2 without the web UI.

## What 2.21a already shipped

- `seasoned-hand --version` / `--help`
- `seasoned-hand project {list,create,archive}`
- `seasoned-hand task {list,show,pause,resume,cancel,provenance}`
- `--server`, `--no-color`, `--json` global flags
- `AppState::register_cli_channel()` + `cli_channel: Arc<CliChannel>` field
- HTTP routes: `GET /v1/projects`, `POST /v1/projects`,
  `POST /v1/projects/:id/archive`, `GET /v1/projects/:id/tasks`,
  `GET /v1/tasks/:id`, `POST /v1/tasks/:id/{pause,resume,cancel}`
  (loopback-only; existing `/v1/tasks/:id/provenance` reused)
- Smoke test scaffolding in `crates/seasoned-hand-cli/tests/cli_smoke.rs`
- DEBT #23 closed

## Acceptance criteria (deferred from 2.21)

### Subcommands
- [ ] `seasoned-hand init` — bootstrap `~/.seasoned-hand/` (config + deliverables dirs)
- [ ] `seasoned-hand server` — exec the `seasoned-hand-server` binary via `std::process::Command::exec` (no in-process reimplementation)
- [ ] `seasoned-hand task new "<brief>" [--project ID] [--detach] [--no-auto-confirm] [--open]`
- [ ] `seasoned-hand task brief <ID>` — print the current `Brief` JSON
- [ ] `seasoned-hand task deliverable <ID> [--open] [--save PATH]`
- [ ] `seasoned-hand inbox` — list pending briefings across all projects
- [ ] `seasoned-hand brief {confirm,edit,cancel} <BRIEFING_ID>`
      - `edit [--editor]` opens `$EDITOR` (default `vi` / `nano` / `vim`) on the current Brief JSON
- [ ] `seasoned-hand channel test <NAME> [--role intake|delivery|notify]` — reuses existing `POST /v1/channels/:name/test`
- [ ] `seasoned-hand channel logs <NAME> [--tail]` — streams via WS subscription

### Server routes
- [ ] `GET /v1/inbox` — pending briefings (`tasks` with status=`briefed` + paginated)
- [ ] `POST /v1/briefings/:id/confirm` body `{action: "confirm" | "edit" | "cancel", edits?: PartialBrief}`
- [ ] `GET /v1/briefings/:id` — single briefing payload (for `brief` subcommand)
- [ ] Loopback-only same as 2.21a; reuse the `require_loopback` helper

### Blocking flow
- [ ] `task new "<brief>"` (default, no `--detach`):
      1. Mints `intake_id` = `cli:<uuid>`
      2. Calls `AppState::cli_channel.register_pending(intake_id)` in-process (when CLI binary is the same `AppState` as the server — see Implementation step 3)
      3. Pushes an `IntakeEvent { channel: "cli", intake_id, brief_input, reply_target: Some(DeliveryTarget { channel: "cli", target_ref: "intake:<intake_id>" }) }`
      4. Awaits the oneshot receiver
      5. Renders the deliverable's rendered_content_path to stdout (text) or calls `open <path>` (`--open`)
- [ ] `--detach` skips the oneshot register/await — fall back to `~/.seasoned-hand/deliverables/<deliverable_id>.<ext>` file
- [ ] `--no-auto-confirm` bypasses the 5-minute auto-confirm (operator drives confirm manually)

## Non-goals (defer to Phase 5)
- `seasoned-hand auth login`
- Per-user CLI config (`~/.seasoned-hand/credentials`)
- Remote CLI talking to a non-loopback server (Phase 5 adds auth; today's `--server URL` only works for loopback)
- TUI / curses UI

---

## Implementation steps

### 1. Server routes
1. `GET /v1/inbox` — query `tasks WHERE status='briefed' ORDER BY created_at DESC`. Return `{briefing_id, task_id, project_id, title, brief, created_at}[]`. The Brief lives in `tasks.brief` (JSON column).
2. `POST /v1/briefings/:id/confirm` — look up the per-task `mpsc::Sender<UserResponse>` from `AppState::briefing_senders`, forward the `UserResponse` envelope. Returns `202 Accepted` on success, `404 no_pending_briefing` when the sender slot is empty (already-confirmed task or terminal).
3. `GET /v1/briefings/:id` — single-row variant of `/v1/inbox`, plus `briefing_call_id` so the CLI can construct the confirm response.

### 2. `task new` blocking flow
The simplest path is to require the CLI binary share an `AppState` with the server (i.e. only works in loopback `--server http://127.0.0.1:3000`). Two options:
- **(a) Talk to the server via HTTP**: CLI sends `POST /v1/intake/cli` with the brief, then polls or subscribes to a WS topic for the deliverable. Requires a new `POST /v1/intake/cli` route + WS topic.
- **(b) In-process when the CLI is the server**: When `seasoned-hand task new` runs INSIDE the same process as `seasoned-hand server` (e.g. a future `seasoned-hand task new --in-process` flag, or a sidecar mode), call `state.cli_channel.register_pending(intake_id)` then push the IntakeEvent directly.

**Recommended**: ship **(a)** in 2.21b because it works against the existing standalone server. Option (b) is a Phase 5 optimization once we have a `seasoned-hand server --foreground` pattern that hosts the CLI alongside.

### 3. `init` subcommand
Mkdir `~/.seasoned-hand/{deliverables,config}`. Idempotent. Optionally write a default `config/notify.toml.example`.

### 4. `server` subcommand
`std::process::Command::new("seasoned-hand-server").args(env::args().skip(1)).exec()` (Unix `CommandExt::exec` replaces the current process — no fork). On Windows fall back to `Command::status()`.

### 5. Smoke test additions
Extend `crates/seasoned-hand-cli/tests/cli_smoke.rs` with:
- `cli_inbox_lists_pending_briefings`
- `cli_brief_confirm_advances_state` (no Initializer spawn — drive the mpsc directly from the test)
- `cli_task_new_detach_writes_fallback_file`

---

## Risk flags

- The 2.21a smoke test left pause / resume happy-paths un-exercised (Docker dependency). 2.21b can pick those back up by mocking the bollard Docker client at the SandboxClient boundary — out of scope for 2.21b unless a happy-path is needed for a different AC.
- `task new` blocking via HTTP needs a brand-new intake surface. Make sure the 4xx surface from `IntakeRouter::handle_event` mirrors `/v1/intake/webhook` exactly (`intake_rejected:<reason>` body shape).

---

## Verification

```bash
cargo build -p seasoned-hand-cli
cargo clippy -p seasoned-hand-cli -p seasoned-hand-server --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-cli
./scripts/spec-check.sh
```

---

## Commit message

```
feat(phase-2): story 2.21b - CLI intake + brief + inbox + init/server

Closes the OS-layer CLI deferred from 2.21a (per the risk-flag split):
task new blocking, brief confirm/edit/cancel, inbox, channel test/
logs, init, server.

- `seasoned-hand task new "<brief>" [--detach] [--open]` blocking
  flow via new `POST /v1/intake/cli` + WS deliverable subscription.
- `seasoned-hand inbox` + `brief {confirm,edit,cancel}` back the
  briefing-confirm gate from story 2.8b without the web UI.
- New routes: GET /v1/inbox, GET /v1/briefings/:id,
  POST /v1/briefings/:id/confirm. Loopback-only (reuses 2.21a's
  require_loopback helper).
- `init` mkdir's ~/.seasoned-hand/{deliverables,config}; `server`
  execs the existing seasoned-hand-server binary.

refs: /specs/phase-2/stories/story-2.21b.md
refs: /specs/phase-2/stories/story-2.21.md
```
