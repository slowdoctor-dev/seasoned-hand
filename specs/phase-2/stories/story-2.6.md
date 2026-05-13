# Story 2.6 — Sandbox-side renderer toolchain

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 2.1
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.3, §5 "Sandbox-side renderer toolchain"

---

## Goal

Make the sandbox able to produce real-employee artifacts: install
Pandoc + python-pptx + openpyxl at session-create time, and ship a
`RendererDispatcher` that takes (source content, target filename) →
writes the rendered artifact into `/workspace/.deliverables/`.

Phase 2 ships the install-at-startup path; phase-2/DEBT.md #2 tracks
the migration to a pre-baked image (deferred to Phase 4).

## Acceptance criteria

- [ ] Sandbox-create path (Phase 0 0.8 / Phase 1 1.2 `SandboxClient::create`)
      gains a renderer-install step **after** workspace git bootstrap
      (Phase 1 1.3) and **before** marking the session ready.
- [ ] Install command: `apt-get install -y pandoc texlive-xetex
      python3-pip && pip3 install python-pptx openpyxl`. Run via
      sandbox `/v1/shell/exec`. Failure on install bubbles as
      `SandboxError::WorkspaceBootstrap` (existing variant), which
      surfaces as `session_create_failed` per Phase 1 conventions.
- [ ] `RendererDispatcher::render(source_content: &[u8],
      target_filename: &str, sandbox: &SandboxClient, session_id: &str)`:
      - Inspects extension of `target_filename`.
      - Routes to one of: `raw_write` (md/txt/json/csv), `pandoc`
        (docx/pdf/html/odt), `python_pptx` (pptx), `openpyxl` (xlsx).
      - Writes source to a temp file in sandbox
        (`/workspace/.deliverables/.source/<uuid>.<src_ext>`), invokes
        the renderer via `/v1/shell/exec`, captures stdout/stderr,
        returns `Result<RenderedArtifact, RenderError>`.
      - `RenderedArtifact { path, size, sha256 }` matches the
        Deliverable column shape.
- [ ] On renderer non-zero exit OR malformed input JSON: returns
      `RenderError::RendererFailed { renderer, exit_code, stderr,
      input_preview }`. The 1-retry "simplify content" path described
      in architecture §8 is NOT in this story — it lands with
      `task_deliver` (story 2.14) which has access to the LLM.
- [ ] Skip-install env override `SANDBOX_SKIP_RENDERER_INSTALL=1` for
      tests + pre-baked-image future. When set, sandbox-create skips
      the install step (assumes the image already has the toolchain).
- [ ] Unit tests:
      - `renderer_raw_writes_unchanged` (md/txt/json/csv pass-through)
      - `renderer_pandoc_markdown_to_docx` (live wiremock'd
        `/v1/shell/exec`)
      - `renderer_pptx_from_json` (wiremock)
      - `renderer_xlsx_from_json` (wiremock)
      - `renderer_dispatches_by_filename_extension`
      - `renderer_failed_exit_returns_error_with_stderr`

## Non-goals

- LLM-driven "simplify and retry" on renderer failure (story 2.14
  with `task_deliver`).
- Pre-baked sandbox image build/publish (phase-2/DEBT #2 — Phase 4).
- The `task_deliver` LLM tool itself (story 2.14).
- Diagram renderers (Graphviz / Mermaid) — phase-2 stretch / Phase 4.

---

## Implementation steps

### 1. Sandbox-create extension

In `crates/seasoned-hand-core/src/sandbox/bootstrap.rs` (new in
Phase 1 1.3 or extended here), add `install_renderer_toolchain(api_url)`.
Called from `SandboxClient::create` after the existing
`run_bootstrap` call. Honors `SANDBOX_SKIP_RENDERER_INSTALL`.

### 2. RendererDispatcher

```
crates/seasoned-hand-core/src/deliverable/renderer/
  mod.rs              ← RendererDispatcher + RenderError + RenderedArtifact
  raw.rs              ← pass-through for md/txt/json/csv
  pandoc.rs           ← markdown → docx/pdf/html/odt
  python_pptx.rs      ← JSON → pptx via inline python script
  openpyxl.rs         ← JSON → xlsx via inline python script
  tests.rs
```

### 3. Python script shape

`python_pptx.rs` and `openpyxl.rs` write a small Python script to a
sandbox tempfile and shell-exec it. Script reads source JSON from
stdin, writes binary output to a path passed as argv. Keeps the
renderer logic versioned with the Rust crate.

### 4. Tests

Each renderer test wiremocks `/v1/shell/exec` to return a canned
exit_code + stdout/stderr. The pandoc test asserts the dispatcher
issued the right command line (`pandoc -f markdown -t docx -o /workspace/.deliverables/<id>.docx`).

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core deliverable::renderer
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/sandbox/bootstrap.rs` (modify — gain
  `install_renderer_toolchain`)
- `crates/seasoned-hand-core/src/sandbox/mod.rs` (modify — call install
  in `create()`)
- `crates/seasoned-hand-core/src/deliverable/renderer/mod.rs` (new)
- `crates/seasoned-hand-core/src/deliverable/renderer/{raw,pandoc,python_pptx,openpyxl,tests}.rs` (new)
- `crates/seasoned-hand-core/src/deliverable/mod.rs` (modify — `pub mod renderer;`)

---

## Spec references

- `/specs/phase-2/architecture.md` §2.3, §5, §7 (per-format perf
  budgets)
- `/specs/phase-2/DEBT.md` #2 (pre-baked image deferral)

---

## Commit message

```
feat(phase-2): story 2.6 - Sandbox-side renderer toolchain (Pandoc + python-pptx + openpyxl)

- SandboxClient::create now installs Pandoc + python-pptx + openpyxl
  after workspace bootstrap, before marking session ready. ~30-60 s
  one-time per session. SANDBOX_SKIP_RENDERER_INSTALL=1 disables
  (tests + pre-baked image future).
- RendererDispatcher routes by target_filename extension:
  raw (md/txt/json/csv), Pandoc (docx/pdf/html/odt), python-pptx
  (pptx), openpyxl (xlsx). Each non-raw renderer writes a tiny
  Python or shell invocation in the sandbox via /v1/shell/exec.
- RenderedArtifact { path, size, sha256 } matches the Deliverable
  column shape.
- RenderError carries renderer + exit_code + stderr + input_preview.
- 6 unit tests with wiremock'd sandbox shell endpoint.

refs: /specs/phase-2/stories/story-2.6.md
```

---

## Notes for next story (2.7)

Renderers are in. 2.7 + 2.8 ship Brief + Initializer confirm gate.
The renderer dispatcher is consumed by `task_deliver` in 2.14.
