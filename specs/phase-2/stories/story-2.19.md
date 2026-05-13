# Story 2.19 — SandboxGitShell shell-injection fix (DEBT #14)

> **Status**: ready
> **Estimated**: 1 hour
> **Dependencies**: —
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-1/DEBT.md` #14

---

## Goal

Pay down Phase 1 DEBT #14 BEFORE the Plan{op:"advance"} broadcaster
activates in Phase 2. Replace
`checkpoint::git_in_sandbox::SandboxGitShell::commit_phase`'s manual
double-quote escaping with a stdin-fed `git commit -F -` so
phase_title never enters the shell context as an interpolated string.

## Acceptance criteria

- [ ] `commit_phase` no longer interpolates `phase_title` into the
      shell command. Instead, writes the commit message to a sandbox
      tempfile (`/tmp/.checkpoint_msg`) via
      `SandboxClient::write_workspace_file` (or its `/tmp` analog —
      add `write_sandbox_file` helper if needed), then invokes
      `git -C /workspace commit -q --allow-empty -F /tmp/.checkpoint_msg`.
- [ ] After commit, the tempfile is deleted via shell-exec
      `rm -f /tmp/.checkpoint_msg`.
- [ ] Regression test `commit_phase_does_not_shell_inject`:
      - Phase titles tested: `` "`whoami`" ``, `$(id)`, `; touch /tmp/pwned`,
        `\n; cat /etc/passwd`
      - Test asserts that NONE of these execute (no `/tmp/pwned` file,
        no extra processes spawned, no command substitution result in
        the commit log)
      - Test uses a real `seasoned-hand-sandbox` container if Docker
        is available; falls back to wiremock'd `/v1/shell/exec` with
        path assertion otherwise (under `#[ignore]`)
- [ ] Existing `RecordingGitShell` test mock (from Phase 1 1.13)
      updates to accept the new shell command shape.
- [ ] Phase 1 `phase-1/DEBT.md` #14 gets strike-through with this
      commit's SHA.

## Non-goals

- Replacing the entire `GitShell` trait abstraction (Phase 1 simplicity
  audit M4 decided to keep it).
- Refactoring the 3-command sequence (`add -A`, `commit`, `rev-parse`)
  — only the commit step needs the injection fix.

---

## Implementation steps

### 1. Add sandbox tempfile helper (if missing)

`SandboxClient::write_workspace_file` already exists (Phase 1 1.2);
extend with `write_sandbox_temp(session_id, name, bytes) -> Result<String, SandboxError>`
that writes to `/tmp/<name>` (NOT `/workspace/...`). Or — simpler —
just use `write_workspace_file` with a path under `/workspace/.commit-msg/`
and adjust the `git -F` argument accordingly.

The cleanest path: write to `/workspace/.commit-msg/<call_id>.txt` via
the existing `write_workspace_file`. Sandbox sees the file, git reads
via `-F`, we `rm -f` afterwards.

### 2. Modify commit_phase

```rust
async fn commit_phase(
    &self,
    session_id: &str,
    phase_id: i64,
    phase_title: &str,
) -> Result<String, CheckpointGitError> {
    let handle = self.sandbox.get(session_id).await
        .ok_or_else(|| CheckpointGitError::NoSandbox(session_id.into()))?;
    let api_url = &handle.api_url;

    // Write commit message to a workspace file
    let msg_path = format!("/workspace/.commit-msg/{phase_id}.txt");
    let msg_body = format!("phase {phase_id}: {phase_title}");
    self.sandbox
        .write_workspace_file(session_id, &msg_path, msg_body.as_bytes())
        .await
        .map_err(|e| CheckpointGitError::Http(e.to_string()))?;

    // git -C /workspace add -A
    self.exec(api_url, "git -C /workspace add -A").await?;
    // git commit reads message from file — no shell interpolation
    self.exec(
        api_url,
        &format!("git -C /workspace commit -q --allow-empty -F {msg_path}"),
    )
    .await?;
    // Clean up
    self.exec(api_url, &format!("rm -f {msg_path}")).await?;

    let head = self.exec(api_url, "git -C /workspace rev-parse HEAD").await?;
    ...
    Ok(head.stdout.trim().to_string())
}
```

(The `msg_path` value is server-generated using a numeric `phase_id`,
so it can't itself carry injection. Belt-and-suspenders: validate
`phase_id` is a plain integer at the type level — it's `i64` already.)

### 3. Regression test

```rust
#[tokio::test]
async fn commit_phase_does_not_shell_inject() {
    let mock = MockServer::start().await;
    // Capture all /v1/shell/exec calls and assert none contain raw injection patterns
    let received_commands = Arc::new(Mutex::new(Vec::new()));
    let received = received_commands.clone();
    Mock::given(method("POST")).and(path("/v1/shell/exec"))
        .respond_with(move |req: &Request| {
            let body: Value = req.body_json().unwrap();
            received.lock().unwrap().push(body["command"].as_str().unwrap().to_string());
            ResponseTemplate::new(200).set_body_json(json!({"exit_code": 0, "stdout": "", "stderr": ""}))
        })
        .mount(&mock).await;

    // ... fixture setup ...

    let nasty_titles = [
        "`whoami`",
        "$(id)",
        "; touch /tmp/pwned",
        "\n; cat /etc/passwd",
    ];
    for title in nasty_titles {
        let shell = SandboxGitShell::new(...);
        shell.commit_phase(&session_id, 42, title).await.unwrap();
    }

    let cmds = received_commands.lock().unwrap().clone();
    for cmd in &cmds {
        assert!(!cmd.contains("`whoami`"), "raw backtick in: {cmd}");
        assert!(!cmd.contains("$(id)"), "raw $() in: {cmd}");
        assert!(!cmd.contains("; touch /tmp/pwned"), "raw injection in: {cmd}");
        // The commit message goes through write_workspace_file, NOT
        // through the shell command. The shell command should reference
        // /workspace/.commit-msg/42.txt instead.
    }
    // Confirm at least one command was a `commit -F /workspace/.commit-msg/...`
    assert!(cmds.iter().any(|c| c.contains("commit -q --allow-empty -F /workspace/.commit-msg/42.txt")));
}
```

### 4. DEBT strike-through

`specs/phase-1/DEBT.md`:
```
### ~~14. SandboxGitShell::commit_phase builds a shell string...~~ ✅ resolved 2026-MM-DD (story 2.19, commit `XXXX`)
```

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core checkpoint::git_in_sandbox
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/checkpoint/git_in_sandbox.rs` (modify
  — replace shell interpolation with `commit -F`)
- `crates/seasoned-hand-core/src/checkpoint/tests.rs` (modify — add
  the regression test + update RecordingGitShell expectations)
- `specs/phase-1/DEBT.md` (modify — strike-through #14)

---

## Spec references

- `/specs/phase-1/DEBT.md` #14
- `/specs/phase-1/stories/story-1.13.md`

---

## Commit message

```
fix(phase-2): story 2.19 - SandboxGitShell shell-injection (DEBT #14 close)

Replace manual double-quote escaping in commit_phase with a stdin-via-
tempfile pattern. The commit message is written via
SandboxClient::write_workspace_file to /workspace/.commit-msg/<phase_id>.txt,
then `git commit -F <path>` reads it. No phase_title content ever
enters the shell context.

- Added regression test commit_phase_does_not_shell_inject with
  payloads `whoami`, $(id), `; touch /tmp/pwned`, newline-prefix
  injection. Asserts none of these strings appear in any
  /v1/shell/exec command body.
- Tempfile path uses server-generated numeric phase_id (i64); no
  user input in the filename.

closes: Phase 1 DEBT #14

refs: /specs/phase-2/stories/story-2.19.md
```

---

## Notes for next story (2.20)

DEBT #14 closes — the Plan{op:"advance"} broadcaster can now safely
activate (Phase 2 wires the broadcaster as part of stories 2.14 +
2.16 +  2.25). 2.20 finishes the DEBT carry-over trifecta with the
NarratorHook classifier-slot wiring.
