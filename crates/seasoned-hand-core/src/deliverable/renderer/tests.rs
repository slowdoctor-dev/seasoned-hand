//! Renderer dispatcher tests.
//!
//! Strategy: wiremock the sandbox's `/v1/shell/exec` and use a fake
//! `SandboxHandle` whose `workspace_host_path` is a tmpdir. The four
//! renderer paths exercise:
//! - raw bytes → workspace fs (no shell-exec)
//! - pandoc shell command shape (wiremock asserts exit_code=0)
//! - python-pptx + openpyxl shell command shape
//! - non-zero exit → `RenderError::RendererFailed`
//! - dispatch routing by filename extension

use std::path::PathBuf;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wiremock::matchers::{method, path as wm_path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{
    DELIVERABLES_DIR, RenderError, RenderedArtifact, Renderer, RendererDispatcher, pick_renderer,
};
use crate::sandbox::{SandboxClient, SandboxHandle};

const SESSION: &str = "renderer-test-session";

/// Boot a SandboxClient with a tmpdir workspace + a wiremock'd
/// `/v1/shell/exec` returning a canned outcome. Returns the
/// dispatcher, the tmpdir guard (drop = cleanup), and the mock server
/// so the test can wire additional matchers.
async fn fixture_with_exit(
    exit_code: i32,
    stderr: &str,
) -> (RendererDispatcher, TempDir, MockServer) {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(wm_path("/v1/shell/exec"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "exit_code": exit_code,
            "stdout": "",
            "stderr": stderr,
        })))
        .mount(&server)
        .await;

    let sandbox =
        SandboxClient::new("ghcr.io/agent-infra/sandbox:test", tmp.path()).expect("client");
    sandbox
        .insert_handle_for_test(SandboxHandle {
            session_id: SESSION.into(),
            container_id: "test-container".into(),
            api_url: server.uri(),
            novnc_url: "http://127.0.0.1:0".into(),
            ttyd_url: "ws://127.0.0.1:0".into(),
            workspace_host_path: tmp.path().to_path_buf(),
        })
        .await;
    let dispatcher = RendererDispatcher::new(Arc::new(sandbox));
    (dispatcher, tmp, server)
}

/// Pre-create the deliverables target on host fs so the renderer's
/// "fingerprint" read finds the file. Non-raw renderers don't
/// actually produce the file in the wiremock'd path — we plant a
/// stand-in here so [`fingerprint_artifact`] reads something.
fn plant_rendered_file(tmp: &TempDir, relative: &str, bytes: &[u8]) -> PathBuf {
    let path = tmp.path().join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(&path, bytes).expect("plant");
    path
}

#[test]
fn renderer_dispatches_by_filename_extension() {
    assert_eq!(pick_renderer("md").unwrap(), Renderer::Raw);
    assert_eq!(pick_renderer("txt").unwrap(), Renderer::Raw);
    assert_eq!(pick_renderer("json").unwrap(), Renderer::Raw);
    assert_eq!(pick_renderer("csv").unwrap(), Renderer::Raw);
    assert_eq!(pick_renderer("docx").unwrap(), Renderer::Pandoc);
    assert_eq!(pick_renderer("pdf").unwrap(), Renderer::Pandoc);
    assert_eq!(pick_renderer("html").unwrap(), Renderer::Pandoc);
    assert_eq!(pick_renderer("odt").unwrap(), Renderer::Pandoc);
    assert_eq!(pick_renderer("pptx").unwrap(), Renderer::PythonPptx);
    assert_eq!(pick_renderer("xlsx").unwrap(), Renderer::Openpyxl);
    assert!(matches!(
        pick_renderer("rtf").unwrap_err(),
        RenderError::UnsupportedExtension(ref e) if e == "rtf"
    ));
}

#[tokio::test]
async fn renderer_raw_writes_unchanged() {
    let (dispatcher, tmp, _server) = fixture_with_exit(0, "").await;
    let source = b"# Hello\n\nMarkdown body.\n";
    let artifact = dispatcher
        .render(source, "report.md", SESSION)
        .await
        .expect("raw render ok");
    assert_eq!(
        artifact.workspace_path,
        format!("{DELIVERABLES_DIR}/report.md")
    );

    // Bytes landed at the host workspace.
    let on_disk = std::fs::read(tmp.path().join(&artifact.workspace_path)).expect("read");
    assert_eq!(on_disk, source);
    // sha256 matches.
    let expected = format!("{:x}", Sha256::digest(source));
    assert_eq!(artifact.sha256, expected);
    assert_eq!(artifact.size as usize, source.len());
}

/// Pandoc dispatch — wiremock returns exit_code=0 for any POST to
/// `/v1/shell/exec`. The renderer plant lands a stand-in file the
/// fingerprint step reads. Command-shape assertion lives in
/// [`pandoc::pandoc_command`]'s unit test below.
#[tokio::test]
async fn renderer_pandoc_markdown_to_docx() {
    let (dispatcher, tmp, _server) = fixture_with_exit(0, "").await;
    plant_rendered_file(
        &tmp,
        &format!("{DELIVERABLES_DIR}/foo.docx"),
        b"<fake-docx-bytes>",
    );
    let artifact = dispatcher
        .render(b"# foo\n", "foo.docx", SESSION)
        .await
        .expect("pandoc render ok");
    assert!(artifact.workspace_path.ends_with("foo.docx"));
    assert_eq!(artifact.size, "<fake-docx-bytes>".len() as u64);
}

#[test]
fn renderer_pandoc_command_shape() {
    use super::pandoc::pandoc_command;
    let cmd = pandoc_command(
        ".deliverables/.source/x.md",
        ".deliverables/foo.docx",
        "docx",
    );
    assert!(cmd.contains("pandoc -f markdown -t docx"));
    assert!(cmd.contains("-o /workspace/.deliverables/foo.docx"));
    assert!(cmd.contains("/workspace/.deliverables/.source/x.md"));
    let pdf = pandoc_command("a.md", "b.pdf", "pdf");
    assert!(pdf.contains("--pdf-engine=xelatex"));
}

#[tokio::test]
async fn renderer_pptx_from_json() {
    let (dispatcher, tmp, _server) = fixture_with_exit(0, "").await;
    plant_rendered_file(
        &tmp,
        &format!("{DELIVERABLES_DIR}/deck.pptx"),
        b"<fake-pptx>",
    );

    let source = br#"{"slides":[{"title":"Hi","body":"There"}]}"#;
    let artifact: RenderedArtifact = dispatcher
        .render(source, "deck.pptx", SESSION)
        .await
        .expect("pptx render ok");
    assert!(artifact.workspace_path.ends_with("deck.pptx"));
}

#[tokio::test]
async fn renderer_xlsx_from_json() {
    let (dispatcher, tmp, _server) = fixture_with_exit(0, "").await;
    plant_rendered_file(
        &tmp,
        &format!("{DELIVERABLES_DIR}/book.xlsx"),
        b"<fake-xlsx>",
    );

    let source = br#"{"sheets":[{"name":"S1","rows":[["a","b"],[1,2]]}]}"#;
    let artifact = dispatcher
        .render(source, "book.xlsx", SESSION)
        .await
        .expect("xlsx render ok");
    assert!(artifact.workspace_path.ends_with("book.xlsx"));
}

#[tokio::test]
async fn renderer_failed_exit_returns_error_with_stderr() {
    let (dispatcher, _tmp, _server) = fixture_with_exit(2, "pandoc: unknown writer 'qbz'").await;
    let err = dispatcher
        .render(b"# foo\n", "report.docx", SESSION)
        .await
        .expect_err("non-zero exit must surface");
    match err {
        RenderError::RendererFailed {
            renderer,
            exit_code,
            stderr,
            input_preview,
        } => {
            assert_eq!(renderer, "pandoc");
            assert_eq!(exit_code, 2);
            assert!(stderr.contains("pandoc"), "stderr captured: {stderr}");
            assert!(input_preview.contains("foo"), "preview carries source");
        }
        other => panic!("expected RendererFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn renderer_pptx_rejects_malformed_json() {
    // No shell-exec needed — the renderer validates JSON shape before
    // touching the sandbox.
    let (dispatcher, _tmp, _server) = fixture_with_exit(0, "").await;
    let err = dispatcher
        .render(b"not-json", "deck.pptx", SESSION)
        .await
        .expect_err("malformed json rejected");
    assert!(
        matches!(err, RenderError::Json(_)),
        "expected Json variant, got {err:?}"
    );
}
