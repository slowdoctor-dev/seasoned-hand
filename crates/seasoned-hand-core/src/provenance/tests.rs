//! Provenance manifest builder + spill + route tests (story 2.15 AC6).
//!
//! refs: /specs/phase-2/stories/story-2.15.md

use std::sync::Arc;

use serde_json::{Value, json};
use tempfile::TempDir;

use super::*;
use crate::channel::IntakeEvent;
use crate::checkpoint::{CheckpointStore, NewCheckpoint};
use crate::db::{self, DbPool};
use crate::deliverable::{DeliverableStore, NewDeliverable};
use crate::delivery::store::{DeliveryEventStore, NewDeliveryEvent};
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::intake::store::IntakeEventStore;
use crate::project::{NewProject, NewTask, ProjectStore, TaskStore};
use crate::sandbox::{SandboxClient, SandboxHandle};
use crate::verifier::{NewVerification, VerdictKind, VerificationStore, VerifyTrigger};

struct Fixture {
    pool: DbPool,
    _tmp: TempDir,
    sandbox: Arc<SandboxClient>,
    task_id: String,
    project_id: String,
    session_ids: Vec<String>,
    events: Arc<SqliteEventStore>,
    intake_store: IntakeEventStore,
    delivery_store: DeliveryEventStore,
    deliverables: DeliverableStore,
    task_store: TaskStore,
    project_store: ProjectStore,
    verifications: VerificationStore,
    checkpoints: CheckpointStore,
}

/// Seed a fixture with `num_sessions` sessions, a project + task, one
/// intake event linked to the task, and a sandbox handle per session so
/// the spill / file-ref tests can do real workspace IO against the
/// temp dir.
async fn seed_fixture(num_sessions: usize) -> Fixture {
    let pool = db::open(":memory:").await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let sandbox = Arc::new(SandboxClient::new("test", tmp.path()).unwrap());

    let project_store = ProjectStore::new(pool.clone());
    let task_store = TaskStore::new(pool.clone());
    let intake_store = IntakeEventStore::new(pool.clone());
    let delivery_store = DeliveryEventStore::new(pool.clone());
    let deliverables = DeliverableStore::new(pool.clone());
    let events = Arc::new(SqliteEventStore::new(pool.clone()));
    let verifications = VerificationStore::new(pool.clone());
    let checkpoints = CheckpointStore::new(pool.clone());

    let project_id = project_store
        .insert(NewProject {
            tenant_id: Some("tenant-a".into()),
            title: "Demo Project".into(),
            description: None,
        })
        .await
        .unwrap();
    let task_id = task_store
        .insert(NewTask {
            project_id: project_id.clone(),
            tenant_id: Some("tenant-a".into()),
            title: "Demo Task".into(),
            expected_due_at: None,
        })
        .await
        .unwrap();

    let intake_event = IntakeEvent {
        channel: "webhook".into(),
        intake_id: "intake-1".into(),
        brief_input: "Build a thing".into(),
        reply_target: None,
        metadata: json!({"source": "test"}),
        tenant_id: Some("tenant-a".into()),
        received_at: 1_000_000,
    };
    let intake_id = intake_store.insert(&intake_event).await.unwrap();
    intake_store
        .link_to_task(&intake_id, &task_id)
        .await
        .unwrap();

    let mut session_ids = Vec::new();
    for i in 0..num_sessions {
        let sid = format!("sess-{i}");
        let started = 10_000_000_i64 + (i as i64) * 2_000_000;
        let ended = started + 1_000_000;
        // Every session except the last is paused (SUSPENDED); the last
        // is the terminal one (FINISHED). Mirrors the §2.6 pause-resume
        // pattern.
        let state_str = if i + 1 == num_sessions {
            "FINISHED"
        } else {
            "SUSPENDED"
        };
        let sid_owned = sid.clone();
        let tid_owned = task_id.clone();
        let state_owned = state_str.to_string();
        pool.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state, task_id, cost_cents) \
                 VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![sid_owned, started, ended, state_owned, tid_owned, 100_i64],
            )
            .unwrap();
        })
        .await;
        sandbox
            .insert_handle_for_test(SandboxHandle {
                session_id: sid.clone(),
                container_id: "c".into(),
                api_url: "http://127.0.0.1:0".into(),
                novnc_url: "http://127.0.0.1:0".into(),
                ttyd_url: "ws://127.0.0.1:0".into(),
                workspace_host_path: tmp.path().to_path_buf(),
            })
            .await;
        session_ids.push(sid);
    }

    Fixture {
        pool,
        _tmp: tmp,
        sandbox,
        task_id,
        project_id,
        session_ids,
        events,
        intake_store,
        delivery_store,
        deliverables,
        task_store,
        project_store,
        verifications,
        checkpoints,
    }
}

fn build_deps<'a>(fx: &'a Fixture) -> BuildDeps<'a> {
    BuildDeps {
        task_store: &fx.task_store,
        project_store: &fx.project_store,
        intake_store: &fx.intake_store,
        delivery_store: &fx.delivery_store,
        events: &fx.events,
        verifications: &fx.verifications,
        checkpoints: &fx.checkpoints,
        db: &fx.pool,
    }
}

async fn emit_misc(events: &SqliteEventStore, session_id: &str, kind: &str) -> i64 {
    events
        .append(NewEvent {
            session_id: session_id.into(),
            event_type: EventType::Misc,
            source: "test".into(),
            data: json!({"kind": kind}),
        })
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn manifest_carries_all_required_fields() {
    let fx = seed_fixture(1).await;
    let sid = fx.session_ids[0].clone();

    // Plant a brief, a couple of decisions, a verifier row, and a
    // checkpoint. The manifest should link every one of them.
    let brief_id = emit_misc(&fx.events, &sid, "briefing").await;
    let dec_a = emit_misc(&fx.events, &sid, "decision").await;
    let dec_b = emit_misc(&fx.events, &sid, "decision").await;

    let verdict_id = fx
        .verifications
        .insert(NewVerification {
            session_id: sid.clone(),
            triggered_at_event_id: 1,
            trigger: VerifyTrigger::TaskComplete {
                final_message_call_id: "c1".into(),
            },
            verdict: VerdictKind::Pass,
            reason: "looks good".into(),
            evidence_event_ids: vec![1, 2],
            suggested_plan_update: None,
            model_id: "model-x".into(),
            cost_cents: 5,
        })
        .await
        .unwrap();
    let checkpoint_id = fx
        .checkpoints
        .insert(NewCheckpoint {
            session_id: sid.clone(),
            plan_phase_id: 0,
            git_sha: "abc1234".into(),
            label: Some("phase-0".into()),
            triggered_by_event_id: 1,
        })
        .await
        .unwrap();

    let citations = vec![1_i64, 2];
    let manifest = build_manifest(
        ManifestInputs {
            task_id: &fx.task_id,
            deliverable_id: "deliv-1",
            rendered_content_sha256: "deadbeef",
            source_content_sha256: Some("cafebabe"),
            citations: &citations,
        },
        &build_deps(&fx),
    )
    .await
    .unwrap();

    assert_eq!(manifest.schema_version, SCHEMA_VERSION);
    assert_eq!(manifest.task_id, fx.task_id);
    assert_eq!(manifest.project_id, fx.project_id);
    assert_eq!(manifest.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(manifest.intake.channel, "webhook");
    assert_eq!(manifest.intake.intake_id, "intake-1");
    assert_eq!(manifest.brief.brief_event_id, Some(brief_id));
    assert_eq!(manifest.sessions.len(), 1);
    assert_eq!(manifest.sessions[0].id, sid);
    assert_eq!(
        manifest.sessions[0].end_reason.as_deref(),
        Some("completed")
    );
    assert_eq!(manifest.decisions, vec![dec_a, dec_b]);
    assert_eq!(manifest.verifier_verdicts, vec![verdict_id]);
    assert_eq!(manifest.checkpoints.len(), 1);
    assert_eq!(manifest.checkpoints[0].checkpoint_id, checkpoint_id);
    assert_eq!(manifest.checkpoints[0].git_sha, "abc1234");
    assert!(!manifest.checkpoints[0].rolled_back);
    assert_eq!(manifest.metrics.sessions_count, 1);
    assert_eq!(manifest.metrics.pause_resume_cycles, 0);
    assert_eq!(manifest.metrics.verifier_runs, 1);
    assert_eq!(manifest.source_content_sha256.as_deref(), Some("cafebabe"));
    assert_eq!(manifest.rendered_content_sha256, "deadbeef");
    assert_eq!(manifest.citations, citations);
    // No delivery yet at build time — overlay happens at route read.
    assert!(manifest.delivered_to.is_empty());
}

#[tokio::test]
async fn manifest_spills_to_file_when_over_100_kb() {
    let fx = seed_fixture(1).await;
    let sid = fx.session_ids[0].clone();
    // Inflate decisions past the 100 KB serialized threshold. Each
    // decision id serializes to ~5.4 bytes (digits + separator) once
    // wrapped in the manifest array; 20 000 entries comfortably clears
    // the 102 400-byte ceiling.
    let mut decision_ids = Vec::with_capacity(20_000);
    for _ in 0..20_000 {
        let id = emit_misc(&fx.events, &sid, "decision").await;
        decision_ids.push(id);
    }

    let manifest = build_manifest(
        ManifestInputs {
            task_id: &fx.task_id,
            deliverable_id: "deliv-1",
            rendered_content_sha256: "abc",
            source_content_sha256: None,
            citations: &[],
        },
        &build_deps(&fx),
    )
    .await
    .unwrap();
    assert_eq!(manifest.decisions.len(), decision_ids.len());

    let serialized_size = serde_json::to_string(&manifest).unwrap().len();
    let column = persist_or_spill(
        &manifest,
        fx.sandbox.as_ref(),
        &sid,
        &fx.task_id,
        INLINE_THRESHOLD_BYTES,
    )
    .await
    .unwrap();
    assert!(
        column.is_file_ref(),
        "manifest must spill past 100 KB (got {} bytes serialized)",
        serialized_size
    );

    let column_value = column.into_column_value();
    let ref_uri = column_value
        .get("$ref")
        .and_then(Value::as_str)
        .expect("file ref");
    assert_eq!(
        ref_uri,
        format!("file:///workspace/.provenance/{}.json", fx.task_id)
    );
    // Confirm the file actually landed in the workspace.
    let on_disk = fx
        .sandbox
        .read_workspace_file(&sid, &format!(".provenance/{}.json", fx.task_id))
        .await
        .unwrap();
    let round_trip: ProvenanceManifest = serde_json::from_slice(&on_disk).unwrap();
    assert_eq!(round_trip, manifest);
}

#[tokio::test]
async fn manifest_handles_multi_session_task() {
    // 3 sessions = 2 pause-resume cycles. Emit a decision into each
    // session so the aggregation crosses session boundaries.
    let fx = seed_fixture(3).await;
    let mut decisions = Vec::new();
    for sid in &fx.session_ids {
        let id = emit_misc(&fx.events, sid, "decision").await;
        decisions.push(id);
    }
    // Plant the briefing in the FIRST session — it should win even
    // though later sessions might also (legitimately) carry briefing
    // events from edit cycles.
    let brief_id = emit_misc(&fx.events, &fx.session_ids[0], "briefing").await;
    // Late "briefing" emitted in a later session must NOT replace the
    // first one.
    let _ = emit_misc(&fx.events, &fx.session_ids[1], "briefing").await;

    let manifest = build_manifest(
        ManifestInputs {
            task_id: &fx.task_id,
            deliverable_id: "deliv-1",
            rendered_content_sha256: "abc",
            source_content_sha256: None,
            citations: &[],
        },
        &build_deps(&fx),
    )
    .await
    .unwrap();

    assert_eq!(manifest.sessions.len(), 3);
    assert_eq!(manifest.metrics.sessions_count, 3);
    assert_eq!(manifest.metrics.pause_resume_cycles, 2);
    assert_eq!(manifest.decisions, decisions);
    assert_eq!(manifest.brief.brief_event_id, Some(brief_id));
    assert_eq!(manifest.sessions[0].end_reason.as_deref(), Some("paused"));
    assert_eq!(manifest.sessions[1].end_reason.as_deref(), Some("paused"));
    assert_eq!(
        manifest.sessions[2].end_reason.as_deref(),
        Some("completed")
    );
}

#[tokio::test]
async fn manifest_empty_decisions_yields_empty_array_not_null() {
    let fx = seed_fixture(1).await;
    let manifest = build_manifest(
        ManifestInputs {
            task_id: &fx.task_id,
            deliverable_id: "deliv-1",
            rendered_content_sha256: "abc",
            source_content_sha256: None,
            citations: &[],
        },
        &build_deps(&fx),
    )
    .await
    .unwrap();
    let v = serde_json::to_value(&manifest).unwrap();
    assert_eq!(
        v.get("decisions"),
        Some(&Value::Array(vec![])),
        "decisions must serialize as [] not null"
    );
    assert_eq!(v.get("citations"), Some(&Value::Array(vec![])));
    assert_eq!(v.get("verifier_verdicts"), Some(&Value::Array(vec![])));
}

/// Insert a Deliverable row with the given manifest column value.
/// Returns the persisted deliverable_id.
async fn insert_deliverable_with_manifest(
    store: &DeliverableStore,
    task_id: &str,
    manifest: Value,
) -> String {
    store
        .insert(NewDeliverable {
            task_id: task_id.into(),
            tenant_id: None,
            format: "md".into(),
            source_content_path: None,
            source_content_sha256: None,
            rendered_content_path: ".deliverables/out.md".into(),
            rendered_content_sha256: "abc".into(),
            content_size: 12,
            citations: None,
            provenance_manifest: manifest,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn route_provenance_returns_manifest() {
    let fx = seed_fixture(1).await;
    let sid = fx.session_ids[0].clone();
    let _ = emit_misc(&fx.events, &sid, "briefing").await;

    let manifest = build_manifest(
        ManifestInputs {
            task_id: &fx.task_id,
            deliverable_id: "deliv-x",
            rendered_content_sha256: "abc",
            source_content_sha256: None,
            citations: &[1, 2],
        },
        &build_deps(&fx),
    )
    .await
    .unwrap();
    let deliverable_id = insert_deliverable_with_manifest(
        &fx.deliverables,
        &fx.task_id,
        serde_json::to_value(&manifest).unwrap(),
    )
    .await;

    // Plant ONE delivery event so the route's overlay is exercised.
    let delivery_id = fx
        .delivery_store
        .insert(NewDeliveryEvent {
            tenant_id: None,
            task_id: fx.task_id.clone(),
            deliverable_id: deliverable_id.clone(),
            channel: "webhook".into(),
            target: crate::channel::DeliveryTarget {
                channel: "webhook".into(),
                target_ref: "url:https://example.test/hook".into(),
                metadata: json!({}),
            },
            ok: true,
            external_id: Some("ext-1".into()),
            error: None,
            delivered_at: 5_000_000,
        })
        .await
        .unwrap();

    let outcome = get_task_provenance(
        &fx.task_id,
        GetTaskProvenanceQuery {
            deliverable_id: None,
        },
        GetTaskProvenanceDeps {
            deliverables: &fx.deliverables,
            delivery_events: &fx.delivery_store,
            sandbox: fx.sandbox.as_ref(),
            db: &fx.pool,
        },
    )
    .await;
    let response = match outcome {
        crate::routes::RouteOutcome::Ok(r) => r,
        other => panic!("expected Ok, got {other:?}"),
    };
    assert_eq!(response.deliverable_id, deliverable_id);
    assert_eq!(response.manifest.task_id, fx.task_id);
    assert_eq!(response.manifest.delivered_to.len(), 1);
    assert_eq!(response.manifest.delivered_to[0].delivery_id, delivery_id);
    assert_eq!(response.manifest.delivered_to[0].channel, "webhook");
    assert!(response.manifest.delivered_to[0].ok);
}

#[tokio::test]
async fn route_provenance_resolves_file_ref() {
    let fx = seed_fixture(1).await;
    let sid = fx.session_ids[0].clone();
    let manifest = build_manifest(
        ManifestInputs {
            task_id: &fx.task_id,
            deliverable_id: "deliv-x",
            rendered_content_sha256: "abc",
            source_content_sha256: None,
            citations: &[],
        },
        &build_deps(&fx),
    )
    .await
    .unwrap();
    // Force spill by setting a tiny threshold.
    let column = persist_or_spill(&manifest, fx.sandbox.as_ref(), &sid, &fx.task_id, 16)
        .await
        .unwrap();
    assert!(column.is_file_ref());
    let deliverable_id =
        insert_deliverable_with_manifest(&fx.deliverables, &fx.task_id, column.into_column_value())
            .await;

    let outcome = get_task_provenance(
        &fx.task_id,
        GetTaskProvenanceQuery {
            deliverable_id: Some(deliverable_id.clone()),
        },
        GetTaskProvenanceDeps {
            deliverables: &fx.deliverables,
            delivery_events: &fx.delivery_store,
            sandbox: fx.sandbox.as_ref(),
            db: &fx.pool,
        },
    )
    .await;
    let response = match outcome {
        crate::routes::RouteOutcome::Ok(r) => r,
        other => panic!("expected Ok, got {other:?}"),
    };
    assert_eq!(response.deliverable_id, deliverable_id);
    assert_eq!(response.manifest.task_id, fx.task_id);
    assert_eq!(response.manifest.schema_version, SCHEMA_VERSION);
}
