# Story 2.15 — Provenance manifest builder + /v1/tasks/:id/provenance

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 2.3, 2.5
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.11 (provenance manifest)

---

## Goal

Build the full provenance manifest at Deliverable persist time, store
inline (or spill to file when >100 KB per architecture §12 q5), expose
via `GET /v1/tasks/:id/provenance`. This is the OS-level "I can always
answer 'where did this come from?'" guarantee made concrete.

## Acceptance criteria

- [ ] `seasoned_hand_core::provenance::ProvenanceManifest` struct
      mirrors architecture §2.11 schema verbatim (schema_version: 1).
- [ ] `provenance::build_manifest(task_id, deliverable_id, deps) ->
      Result<ProvenanceManifest, ProvenanceError>`:
      - Loads Task + Project + Tenant (from stores)
      - Loads originating IntakeEvent (single-row lookup by `task_id`)
      - Loads Brief event (Misc kind="briefing" event_id; capture the
        first one — re-emits during edit cycles carry their own ids
        in `edits_applied`)
      - Loads Sessions for the task (one row per pause-resume cycle)
      - Loads decision events: `Misc{kind:"decision"}` for the task
        (cross-session — query by task_id via session_id link)
      - Loads verifier verdicts: VerificationStore rows for the task's
        sessions
      - Loads checkpoints: CheckpointStore rows for the task's sessions
      - Aggregates metrics: tool_calls (sum of Action events),
        cost_cents (latest CostClient snapshot delta from baseline),
        wall_seconds (max session.ended_at - min session.started_at),
        pause_resume_cycles (len(sessions) - 1),
        verifier_runs (len(verdicts))
      - Loads DeliveryEvents for the deliverable (populate
        `delivered_to`)
- [ ] **Size budget**: if serialized JSON > 100 KB, spill to
      `/workspace/.provenance/<task_id>.json` via SandboxClient and
      store `{"$ref": "file:///workspace/.provenance/<task_id>.json"}`
      in the `deliverables.provenance_manifest` column. The route
      handler transparently resolves either inline or file-ref.
- [ ] `task_deliver` tool (story 2.14) — replace the stub manifest
      with a call to `build_manifest`. Manifest is built BEFORE the
      Deliverable row is inserted so the column is populated correctly.
- [ ] Route `GET /v1/tasks/:id/provenance`:
      - Loads the Task's latest Deliverable (most recent
        created_at); returns its manifest.
      - If `?deliverable_id=...` query is supplied, returns that
        specific deliverable's manifest.
      - Returns `RouteOutcome::Ok(manifest)` or `NotFound` /
        `Internal`.
      - Resolves file-ref manifests transparently.
- [ ] Unit tests:
      - `manifest_carries_all_required_fields` (golden-file test)
      - `manifest_spills_to_file_when_over_100_kb`
      - `manifest_handles_multi_session_task` (pause-resume cycle
        counted correctly)
      - `manifest_empty_decisions_yields_empty_array_not_null`
      - `route_provenance_returns_manifest`
      - `route_provenance_resolves_file_ref`

## Non-goals

- Encryption of manifests (Phase 5 — phase-2/DEBT.md tracks).
- Live manifest streaming during task execution (Phase 4 if needed —
  Phase 2 builds at deliverable-persist time only).

---

## Implementation steps

### 1. Module

```
crates/seasoned-hand-core/src/provenance/
  mod.rs
  manifest.rs       ← ProvenanceManifest struct + serde
  builder.rs        ← build_manifest + dep loading
  spill.rs          ← 100 KB threshold + file-ref logic
  tests.rs
```

### 2. Builder

```rust
pub struct BuildDeps<'a> {
    pub task_store: &'a TaskStore,
    pub project_store: &'a ProjectStore,
    pub intake_store: &'a IntakeEventStore,
    pub delivery_store: &'a DeliveryEventStore,
    pub events: &'a SqliteEventStore,
    pub verifications: &'a VerificationStore,
    pub checkpoints: &'a CheckpointStore,
    pub sandbox: Arc<SandboxClient>,
}

pub async fn build_manifest(
    task_id: &str,
    deliverable_id: &str,
    deps: BuildDeps<'_>,
) -> Result<ProvenanceManifest, ProvenanceError>;
```

### 3. Spill helper

```rust
pub async fn persist_or_spill(
    manifest: &ProvenanceManifest,
    sandbox: &SandboxClient,
    session_id: &str,
    threshold: usize,  // default 100 * 1024
) -> Result<ManifestColumn, ProvenanceError>;

pub enum ManifestColumn {
    Inline(String),      // JSON string
    FileRef(String),     // workspace path
}
```

### 4. Route

`crates/seasoned-hand-server/src/lib.rs`:
```rust
async fn get_task_provenance_handler(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Query(q): Query<ProvenanceQuery>,
) -> Result<axum::response::Response, ...> {
    render_outcome("get_task_provenance", get_task_provenance(...))
}
```

### 5. task_deliver integration

Update story 2.14's handler: build manifest, pass to
`DeliverableStore::insert`, which now accepts a manifest column value.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core provenance::
cargo test -p seasoned-hand-server --lib
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/provenance/{mod,manifest,builder,spill,tests}.rs` (new)
- `crates/seasoned-hand-core/src/lib.rs` (modify — `pub mod provenance;`)
- `crates/seasoned-hand-core/src/deliverable/task_deliver.rs` (modify
  — call `build_manifest` instead of stub)
- `crates/seasoned-hand-server/src/lib.rs` (modify — new route)

---

## Spec references

- `/specs/phase-2/architecture.md` §2.11 (manifest schema), §3 V007
  (column), §4 (route), §12 q5 (spill threshold)

---

## Commit message

```
feat(phase-2): story 2.15 - Provenance manifest builder + /v1/tasks/:id/provenance

- ProvenanceManifest struct mirrors architecture §2.11. Built at
  deliverable-persist time (story 2.14 task_deliver handler now
  invokes build_manifest before persisting the row).
- build_manifest loads task + project + intake + brief event + sessions
  + decisions + verifier verdicts + checkpoints + metrics + delivery
  events.
- Spill: >100 KB serialized JSON gets written to
  /workspace/.provenance/<task_id>.json; the column stores a
  file-ref ({"$ref": ...}). Transparent resolution at read time.
- GET /v1/tasks/:id/provenance returns the latest deliverable's
  manifest (or a specific one with ?deliverable_id=...). Uses shared
  RouteOutcome.
- 6 unit tests including golden-file + multi-session + spill.

refs: /specs/phase-2/stories/story-2.15.md
```

---

## Notes for next story (2.16)

Provenance is mandatory now. 2.16 makes the 24h+ pause/resume work
even when the sandbox container has been GC'd between cycles.
