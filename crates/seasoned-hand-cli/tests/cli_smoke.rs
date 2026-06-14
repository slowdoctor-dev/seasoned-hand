//! Story 2.21a — `seasoned-hand` CLI smoke test.
//!
//! Spawns the production [`seasoned_hand_server::app`] router in-process
//! against an in-memory SQLite + unreachable redis (PRINCIPLE #10
//! failure-tolerance), then invokes the CLI binary out-of-process via
//! `CARGO_BIN_EXE_seasoned-hand` so we exercise the real
//! clap → reqwest → server roundtrip. The sandbox client is never
//! contacted because every assertion stays on the project / task /
//! cancel-without-session paths.
//!
//! refs: /specs/phase-2/stories/story-2.21.md

use std::net::SocketAddr;
use std::process::Command;

use seasoned_hand_core::audit::{AuditAction, AuditLogger, AuditRecord};
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use serde_json::Value;
use tokio::net::TcpListener;

struct Harness {
    server_url: String,
    state: AppState,
}

async fn boot() -> Harness {
    let pool = db::open(":memory:").await.expect("db open");
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").expect("redis url");
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .expect("sandbox client");
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::default_for_bifrost();
    let state = AppState::new(pool, redis, sandbox, search, router, Default::default())
        .register_cli_channel()
        // Issue #7 / ADR-018: the CLI authenticates via x-seasoned-hand-* headers
        // (SH_* env), which are accepted only under the insecure-headers flag.
        .allow_insecure_auth_headers();

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let serve_state = state.clone();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app(serve_state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("serve");
    });

    Harness {
        server_url: format!("http://{addr}"),
        state,
    }
}

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_seasoned-hand"))
}

fn run(mut cli: Command) -> (std::process::ExitStatus, String, String) {
    let output = cli.output().expect("cli spawn");
    (
        output.status,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn apply_auth_env(cmd: &mut Command) {
    cmd.env("SH_TENANT_ID", "tenant-a")
        .env("SH_ORGANIZATION_ID", "org-tenant-a")
        .env("SH_ACTOR_USER_ID", "u-admin")
        .env("SH_ORG_ROLE", "admin");
}

fn apply_auth_env_as(cmd: &mut Command, tenant: &str, org: &str, user: &str, role: &str) {
    cmd.env("SH_TENANT_ID", tenant)
        .env("SH_ORGANIZATION_ID", org)
        .env("SH_ACTOR_USER_ID", user)
        .env("SH_ORG_ROLE", role);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_help_works() {
    let mut cmd = cli();
    cmd.arg("--help");
    let (status, stdout, _) = run(cmd);
    assert!(status.success(), "help exits 0");
    assert!(stdout.contains("project"), "lists project subcommand");
    assert!(stdout.contains("task"), "lists task subcommand");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_project_create_then_list_then_show() {
    let h = boot().await;

    // 1. Create a project.
    let mut cmd = cli();
    cmd.args([
        "--server",
        &h.server_url,
        "--json",
        "--no-color",
        "project",
        "create",
        "Smoke Project",
        "--description",
        "smoke test",
    ]);
    let (status, stdout, stderr) = run(cmd);
    assert!(
        status.success(),
        "project create succeeds; stderr: {stderr}"
    );
    let created: Value = serde_json::from_str(&stdout).expect("project create json");
    let project_id = created["id"].as_str().expect("project id").to_string();
    assert_eq!(created["title"], "Smoke Project");
    assert_eq!(created["status"], "active");

    // 2. List projects (--json).
    let mut cmd = cli();
    cmd.args([
        "--server",
        &h.server_url,
        "--json",
        "--no-color",
        "project",
        "list",
    ]);
    let (status, stdout, _) = run(cmd);
    assert!(status.success());
    let projects: Vec<Value> = serde_json::from_str(&stdout).expect("project list json");
    assert!(
        projects.iter().any(|p| p["id"] == project_id),
        "list includes new project"
    );

    // 3. Archive then verify status flipped.
    let mut cmd = cli();
    cmd.args([
        "--server",
        &h.server_url,
        "--no-color",
        "project",
        "archive",
        &project_id,
    ]);
    let (status, _, stderr) = run(cmd);
    assert!(status.success(), "archive exits 0; stderr: {stderr}");
    let stored = h
        .state
        .projects
        .get(&project_id)
        .await
        .expect("get project");
    assert_eq!(
        stored.status,
        seasoned_hand_core::project::ProjectStatus::Archived
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_task_show_and_list_against_seeded_db() {
    let h = boot().await;

    // Seed a project + drafted task directly through the store APIs —
    // the agent runner never spawns so we stay deterministic.
    let project_id = h
        .state
        .projects
        .insert(seasoned_hand_core::project::NewProject {
            tenant_id: None,
            title: "Inbox".into(),
            description: None,
        })
        .await
        .expect("insert project");
    let task_id = h
        .state
        .tasks
        .insert(seasoned_hand_core::project::NewTask {
            project_id: project_id.clone(),
            tenant_id: None,
            title: "summarize last week".into(),
            expected_due_at: None,
        })
        .await
        .expect("insert task");

    // task show --json round-trips the row.
    let mut cmd = cli();
    cmd.args([
        "--server",
        &h.server_url,
        "--json",
        "--no-color",
        "task",
        "show",
        &task_id,
    ]);
    let (status, stdout, _) = run(cmd);
    assert!(status.success());
    let body: Value = serde_json::from_str(&stdout).expect("task show json");
    assert_eq!(body["id"], task_id);
    assert_eq!(body["status"], "drafted");

    // task list --project <id> returns the seeded task.
    let mut cmd = cli();
    cmd.args([
        "--server",
        &h.server_url,
        "--json",
        "--no-color",
        "task",
        "list",
        "--project",
        &project_id,
    ]);
    let (status, stdout, _) = run(cmd);
    assert!(status.success());
    let body: Vec<Value> = serde_json::from_str(&stdout).expect("task list json");
    assert!(body.iter().any(|t| t["id"] == task_id));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_task_handoff_succeeds_and_prints_audit_id() {
    let h = boot().await;
    h.state
        .db
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
                 VALUES ('org-tenant-a', 'tenant-a', 'tenant-a', 'Tenant A', 'active', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
                 VALUES ('u-admin', 'tenant-a', 'admin@acme.dev', 'Admin', 'active', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
                 VALUES ('u-old', 'tenant-a', 'old@acme.dev', 'Old', 'active', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
                 VALUES ('u-target', 'tenant-a', 'target@acme.dev', 'Target', 'active', 1, 1)",
                [],
            )?;
            Ok::<(), rusqlite::Error>(())
        })
        .await
        .expect("seed auth users");

    let project_id = h
        .state
        .projects
        .insert(seasoned_hand_core::project::NewProject {
            tenant_id: Some("tenant-a".into()),
            title: "handoff".into(),
            description: None,
        })
        .await
        .expect("project");
    let task_id = h
        .state
        .tasks
        .insert(seasoned_hand_core::project::NewTask {
            project_id,
            tenant_id: Some("tenant-a".into()),
            title: "handoff me".into(),
            expected_due_at: None,
        })
        .await
        .expect("task");
    h.state
        .db
        .with_conn({
            let task_id = task_id.clone();
            move |conn| {
                conn.execute(
                    "UPDATE tasks SET owner_user_id = 'u-old' WHERE id = ?",
                    rusqlite::params![task_id],
                )?;
                Ok::<(), rusqlite::Error>(())
            }
        })
        .await
        .expect("owner seed");

    let mut cmd = cli();
    cmd.args([
        "--server",
        &h.server_url,
        "--no-color",
        "task",
        "handoff",
        &task_id,
        "--to",
        "target@acme.dev",
        "--reason",
        "coverage",
    ]);
    apply_auth_env(&mut cmd);
    let (status, stdout, stderr) = run(cmd);
    assert!(status.success(), "handoff failed: {stderr}");
    assert!(stdout.contains("audit="), "expected audit id in output");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_audit_list_returns_json_rows() {
    let h = boot().await;
    h.state
        .db
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
                 VALUES ('org-tenant-a', 'tenant-a', 'tenant-a', 'Tenant A', 'active', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
                 VALUES ('u-admin', 'tenant-a', 'admin@acme.dev', 'Admin', 'active', 1, 1)",
                [],
            )?;
            Ok::<(), rusqlite::Error>(())
        })
        .await
        .expect("seed auth users");

    let logger = AuditLogger::new(h.state.db.clone(), h.state.events.clone());
    logger
        .record(
            &seasoned_hand_core::auth::AuthContext {
                tenant_id: "tenant-a".into(),
                organization_id: "org-tenant-a".into(),
                actor_user_id: "u-admin".into(),
                org_role: seasoned_hand_core::auth::Role::Admin,
                project_override_role: None,
            },
            AuditRecord {
                action: AuditAction::TaskHandoff,
                resource_type: "task",
                resource_id: "task-x",
                target_user_id: None,
                decision: Some("allow"),
                reason: Some("seed"),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .expect("seed audit row");

    let mut cmd = cli();
    cmd.args([
        "--server",
        &h.server_url,
        "--json",
        "audit",
        "list",
        "--limit",
        "10",
    ]);
    apply_auth_env(&mut cmd);
    let (status, stdout, stderr) = run(cmd);
    assert!(status.success(), "audit list failed: {stderr}");
    let rows: Vec<Value> = serde_json::from_str(&stdout).expect("audit list json");
    assert!(!rows.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_task_cancel_drafted_succeeds() {
    let h = boot().await;
    let project_id = h
        .state
        .projects
        .insert(seasoned_hand_core::project::NewProject {
            tenant_id: None,
            title: "Inbox".into(),
            description: None,
        })
        .await
        .expect("insert project");
    let task_id = h
        .state
        .tasks
        .insert(seasoned_hand_core::project::NewTask {
            project_id,
            tenant_id: None,
            title: "drafted cancel".into(),
            expected_due_at: None,
        })
        .await
        .expect("insert task");

    let mut cmd = cli();
    cmd.args([
        "--server",
        &h.server_url,
        "--no-color",
        "task",
        "cancel",
        &task_id,
    ]);
    let (status, _, stderr) = run(cmd);
    assert!(
        status.success(),
        "drafted → cancelled is legal; stderr: {stderr}"
    );

    let task = h.state.tasks.get(&task_id).await.expect("get task");
    assert_eq!(
        task.status,
        seasoned_hand_core::project::TaskStatus::Cancelled
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_task_cancel_terminal_returns_409() {
    let h = boot().await;
    let project_id = h
        .state
        .projects
        .insert(seasoned_hand_core::project::NewProject {
            tenant_id: None,
            title: "Inbox".into(),
            description: None,
        })
        .await
        .expect("insert project");
    let task_id = h
        .state
        .tasks
        .insert(seasoned_hand_core::project::NewTask {
            project_id,
            tenant_id: None,
            title: "already-done".into(),
            expected_due_at: None,
        })
        .await
        .expect("insert task");
    // Walk Drafted → Briefed → Confirmed → Cancelled (terminal).
    h.state
        .tasks
        .set_status(&task_id, seasoned_hand_core::project::TaskStatus::Briefed)
        .await
        .unwrap();
    h.state
        .tasks
        .set_status(&task_id, seasoned_hand_core::project::TaskStatus::Confirmed)
        .await
        .unwrap();
    h.state
        .tasks
        .set_status(&task_id, seasoned_hand_core::project::TaskStatus::Cancelled)
        .await
        .unwrap();

    let mut cmd = cli();
    cmd.args([
        "--server",
        &h.server_url,
        "--no-color",
        "task",
        "cancel",
        &task_id,
    ]);
    let (status, _, stderr) = run(cmd);
    assert!(
        !status.success(),
        "cancel on terminal task should fail; stderr was: {stderr}"
    );
    assert!(
        stderr.contains("wrong_state") || stderr.contains("409"),
        "stderr surfaces wrong-state: {stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_task_pause_without_session_returns_conflict() {
    let h = boot().await;
    let project_id = h
        .state
        .projects
        .insert(seasoned_hand_core::project::NewProject {
            tenant_id: None,
            title: "Inbox".into(),
            description: None,
        })
        .await
        .expect("insert project");
    let task_id = h
        .state
        .tasks
        .insert(seasoned_hand_core::project::NewTask {
            project_id,
            tenant_id: None,
            title: "no session".into(),
            expected_due_at: None,
        })
        .await
        .expect("insert task");

    let mut cmd = cli();
    cmd.args([
        "--server",
        &h.server_url,
        "--no-color",
        "task",
        "pause",
        &task_id,
    ]);
    let (status, _, stderr) = run(cmd);
    assert!(
        !status.success(),
        "pause without session must fail (no row to drive); stderr was: {stderr}"
    );
    assert!(
        stderr.contains("no_active_session") || stderr.contains("409"),
        "stderr surfaces no_active_session: {stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_task_show_404_for_unknown_id() {
    let h = boot().await;
    let mut cmd = cli();
    cmd.args([
        "--server",
        &h.server_url,
        "--no-color",
        "task",
        "show",
        "no-such-task",
    ]);
    let (status, _, stderr) = run(cmd);
    assert!(!status.success());
    assert!(
        stderr.contains("task_not_found") || stderr.contains("404"),
        "stderr surfaces task_not_found: {stderr}"
    );
}

// ---------------------------------------------------------------------
// Story 2.21b — inbox + briefing-confirm + task new --detach + init.
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_inbox_lists_briefed_tasks() {
    let h = boot().await;
    // Seed a project + task, then walk Drafted → Briefed and stash a
    // Brief on the row so the inbox surfaces it.
    let project_id = h
        .state
        .projects
        .insert(seasoned_hand_core::project::NewProject {
            tenant_id: None,
            title: "Inbox".into(),
            description: None,
        })
        .await
        .expect("insert project");
    let task_id = h
        .state
        .tasks
        .insert(seasoned_hand_core::project::NewTask {
            project_id: project_id.clone(),
            tenant_id: None,
            title: "fix the typo".into(),
            expected_due_at: None,
        })
        .await
        .expect("insert task");
    let brief = serde_json::json!({
        "goal": "fix the typo",
        "phases": [{"id": 1, "title": "Plan"}],
    });
    h.state
        .tasks
        .set_brief(&task_id, &brief)
        .await
        .expect("set brief");
    h.state
        .tasks
        .set_status(&task_id, seasoned_hand_core::project::TaskStatus::Briefed)
        .await
        .expect("→ briefed");

    let mut cmd = cli();
    cmd.args(["--server", &h.server_url, "--json", "--no-color", "inbox"]);
    let (status, stdout, stderr) = run(cmd);
    assert!(status.success(), "inbox exits 0; stderr: {stderr}");
    let entries: Vec<Value> = serde_json::from_str(&stdout).expect("inbox json");
    let entry = entries
        .iter()
        .find(|e| e["task_id"] == task_id)
        .expect("seeded task in inbox");
    assert_eq!(entry["briefing_id"], task_id, "briefing_id alias = task_id");
    assert_eq!(entry["project_id"], project_id);
    assert_eq!(entry["title"], "fix the typo");
    assert_eq!(entry["brief"]["goal"], "fix the typo");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_brief_confirm_routes_to_sender() {
    let h = boot().await;
    // The Initializer's spawner is normally what pushes the per-task
    // mpsc sender into AppState::briefing_senders. For this test we
    // simulate that side-channel directly: grab a sender, stash it,
    // fire `brief confirm <id>`, then assert the matching response
    // landed on the receiver.
    let project_id = h
        .state
        .projects
        .insert(seasoned_hand_core::project::NewProject {
            tenant_id: None,
            title: "Inbox".into(),
            description: None,
        })
        .await
        .expect("insert project");
    let task_id = h
        .state
        .tasks
        .insert(seasoned_hand_core::project::NewTask {
            project_id,
            tenant_id: None,
            title: "needs confirm".into(),
            expected_due_at: None,
        })
        .await
        .expect("insert task");
    let (tx, mut rx) =
        tokio::sync::mpsc::channel::<seasoned_hand_core::agent::init::briefing::UserResponse>(4);
    h.state.briefing_senders.insert(task_id.clone(), tx);

    let mut cmd = cli();
    cmd.args([
        "--server",
        &h.server_url,
        "--no-color",
        "brief",
        "confirm",
        &task_id,
    ]);
    let (status, _stdout, stderr) = run(cmd);
    assert!(status.success(), "brief confirm exits 0; stderr: {stderr}");

    let response = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("recv within 2s")
        .expect("sender alive");
    assert!(matches!(
        response.action,
        seasoned_hand_core::agent::init::briefing::BriefingAction::Confirm
    ));
    assert_eq!(response.in_reply_to_call_id, task_id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_brief_confirm_returns_404_when_no_sender() {
    let h = boot().await;
    // No briefing sender registered → 404 no_pending_briefing.
    let mut cmd = cli();
    cmd.args([
        "--server",
        &h.server_url,
        "--no-color",
        "brief",
        "confirm",
        "unknown-task",
    ]);
    let (status, _, stderr) = run(cmd);
    assert!(!status.success());
    assert!(
        stderr.contains("no_pending_briefing") || stderr.contains("404"),
        "stderr surfaces no_pending_briefing: {stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_task_new_detach_returns_intake_id() {
    let h = boot().await;
    let mut cmd = cli();
    cmd.args([
        "--server",
        &h.server_url,
        "--json",
        "--no-color",
        "task",
        "new",
        "Summarise the report",
        "--detach",
    ]);
    let (status, stdout, stderr) = run(cmd);
    assert!(
        status.success(),
        "task new --detach exits 0; stderr: {stderr}"
    );
    let ack: Value = serde_json::from_str(&stdout).expect("ack json");
    assert!(ack["task_id"].as_str().is_some(), "task_id present");
    assert!(
        ack["intake_id"].as_str().unwrap_or("").starts_with("cli:"),
        "intake_id starts with cli: prefix, got {}",
        ack["intake_id"]
    );
    assert_eq!(ack["detached"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_init_creates_dirs() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let mut cmd = cli();
    cmd.env("HOME", tmp.path());
    cmd.args(["--no-color", "init"]);
    let (status, _stdout, stderr) = run(cmd);
    assert!(status.success(), "init exits 0; stderr: {stderr}");
    let root = tmp.path().join(".seasoned-hand");
    assert!(root.exists(), "root dir created");
    assert!(
        root.join("deliverables").exists(),
        "deliverables dir created"
    );
    assert!(root.join("config").exists(), "config dir created");
    // Idempotent — second run shouldn't error.
    let mut cmd = cli();
    cmd.env("HOME", tmp.path());
    cmd.args(["--no-color", "init"]);
    let (status, _, stderr) = run(cmd);
    assert!(status.success(), "second init still 0; stderr: {stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_user_cost_reconcile_smoke() {
    let h = boot().await;
    h.state
        .db
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
                 VALUES ('org-a', 'tenant-a', 'org-a', 'Org A', 'active', 0, 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
                 VALUES ('u-admin', 'tenant-a', 'admin@example.com', 'Admin', 'active', 0, 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO organization_memberships (id, tenant_id, organization_id, user_id, role, is_primary, created_at, updated_at)
                 VALUES ('mem-a', 'tenant-a', 'org-a', 'u-admin', 'admin', 1, 0, 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at)
                 VALUES ('proj-a', 'tenant-a', 'P', 'active', 0, 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO tasks (id, project_id, tenant_id, title, status, created_at, updated_at)
                 VALUES ('task-a', 'proj-a', 'tenant-a', 'T', 'Completed', 0, 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state, project_id, user_id, task_id, cost_cents, tool_calls)
                 VALUES ('sess-a', 1747296000000000, 1747296000000000, 'FINISHED', 'proj-a', 'u-admin', 'task-a', 120, 4)",
                [],
            )?;
            Ok::<(), rusqlite::Error>(())
        })
        .await
        .expect("seed");
    let writer = seasoned_hand_core::billing::NearlineWriter::new(h.state.db.clone());
    writer.flush().await.expect("flush");

    let mut cmd = cli();
    apply_auth_env_as(&mut cmd, "tenant-a", "org-a", "u-admin", "admin");
    cmd.args([
        "--server",
        &h.server_url,
        "--json",
        "--no-color",
        "user-cost",
        "reconcile",
        "--month",
        "2025-05",
    ]);
    let (status, stdout, stderr) = run(cmd);
    assert!(status.success(), "reconcile exits 0; stderr: {stderr}");
    let report: Value = serde_json::from_str(&stdout).expect("reconcile json");
    assert!(report["rows_checked"].as_u64().is_some());
    assert!(report["drifted_rows"].as_u64().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_invite_happy_path() {
    let h = boot().await;
    h.state
        .db
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
                 VALUES ('org-a', 'tenant-a', 'acme', 'Acme', 'active', 0, 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
                 VALUES ('u-admin', 'tenant-a', 'admin@acme.com', 'Admin', 'active', 0, 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO organization_memberships (id, tenant_id, organization_id, user_id, role, is_primary, created_at, updated_at)
                 VALUES ('m-admin', 'tenant-a', 'org-a', 'u-admin', 'admin', 1, 0, 0)",
                [],
            )?;
            Ok::<(), rusqlite::Error>(())
        })
        .await
        .expect("seed");

    let mut cmd = cli();
    apply_auth_env_as(&mut cmd, "tenant-a", "org-a", "u-admin", "admin");
    cmd.args([
        "--server",
        &h.server_url,
        "--json",
        "--no-color",
        "user",
        "invite",
        "new@acme.com",
        "--org",
        "acme",
        "--role",
        "viewer",
    ]);
    let (status, stdout, stderr) = run(cmd);
    assert!(status.success(), "invite exits 0; stderr: {stderr}");
    let out: Value = serde_json::from_str(&stdout).expect("invite json");
    assert!(out["user_id"].as_str().unwrap_or("").starts_with("user-"));
    assert!(!out["login_token"].as_str().unwrap_or("").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_invite_viewer_denied() {
    let h = boot().await;
    h.state
        .db
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
                 VALUES ('org-a', 'tenant-a', 'acme', 'Acme', 'active', 0, 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
                 VALUES ('u-viewer', 'tenant-a', 'viewer@acme.com', 'Viewer', 'active', 0, 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO organization_memberships (id, tenant_id, organization_id, user_id, role, is_primary, created_at, updated_at)
                 VALUES ('m-viewer', 'tenant-a', 'org-a', 'u-viewer', 'viewer', 1, 0, 0)",
                [],
            )?;
            Ok::<(), rusqlite::Error>(())
        })
        .await
        .expect("seed");

    let mut cmd = cli();
    apply_auth_env_as(&mut cmd, "tenant-a", "org-a", "u-viewer", "viewer");
    cmd.args([
        "--server",
        &h.server_url,
        "--json",
        "--no-color",
        "user",
        "invite",
        "nope@acme.com",
        "--org",
        "acme",
        "--role",
        "user",
    ]);
    let (status, _stdout, stderr) = run(cmd);
    assert!(!status.success(), "viewer invite should fail");
    assert!(stderr.contains("forbidden_action"), "stderr={stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_invite_cross_org_admin_denied() {
    let h = boot().await;
    h.state
        .db
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
                 VALUES ('org-a', 'tenant-a', 'acme', 'Acme', 'active', 0, 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
                 VALUES ('org-b', 'tenant-b', 'beta', 'Beta', 'active', 0, 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
                 VALUES ('u-admin', 'tenant-a', 'admin@acme.com', 'Admin', 'active', 0, 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO organization_memberships (id, tenant_id, organization_id, user_id, role, is_primary, created_at, updated_at)
                 VALUES ('m-admin', 'tenant-a', 'org-a', 'u-admin', 'admin', 1, 0, 0)",
                [],
            )?;
            Ok::<(), rusqlite::Error>(())
        })
        .await
        .expect("seed");

    let mut cmd = cli();
    apply_auth_env(&mut cmd);
    cmd.args([
        "--server",
        &h.server_url,
        "--json",
        "--no-color",
        "user",
        "invite",
        "x@beta.com",
        "--org",
        "beta",
        "--role",
        "user",
    ]);
    let (status, _stdout, stderr) = run(cmd);
    assert!(!status.success(), "cross-org invite should fail");
    assert!(stderr.contains("cross_tenant_denied"), "stderr={stderr}");
}
