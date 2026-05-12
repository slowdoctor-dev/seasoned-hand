# Story 1.14 — Hook output-truncation → sandbox file-ref path (close DEBT #21)

> **Status**: ready
> **Estimated**: 1.5 hours
> **Dependencies**: 1.3 (sandbox workspace exists for file_ref writes)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-0/DEBT.md` #21 (origin), `/specs/phase-1/architecture.md`
> §6 row "Hook output-truncation (DEBT #21)", `/specs/phase-1/stories/story-1.16.md`
> (downstream consumer — Track C screenshots).

---

## Goal

Replace Phase 0's "inline preview fallback" path with a real sandbox-file-
write path for any hook output that exceeds the 16 KB inline cap. After
this story, the event stream stores a `file_ref` (path inside
`/workspace/.eventfiles/<event_id>.bin` plus content-type) instead of an
inline preview when content is large. Unblocks story 1.16 (3-track
screenshots routinely exceed 16 KB) and improves Verifier evidence
faithfulness.

## Acceptance criteria

- [ ] `seasoned-hand-core::events::truncation::write_large_or_inline`
      receives `(session_id, event_id, bytes, content_type)` and returns
      `EventPayloadBody::Inline { bytes } | EventPayloadBody::FileRef
      { path, content_type, sha256, size }`. Threshold = 16 KB
      (`INLINE_CAP_BYTES = 16 * 1024`).
- [ ] Large payloads write to `/workspace/.eventfiles/<event_id>.<ext>`
      via the SandboxClient. Extension derived from content-type via a
      small table (`text/plain → .txt`, `application/json → .json`,
      `image/png → .png`, default `.bin`).
- [ ] All existing hook emit-sites (story 0.10's EventEmittingHook —
      Action + Observation paths) are routed through this helper.
      Phase 0's inline-preview branch is removed entirely (no
      backwards-compat shim — clean replacement).
- [ ] `EventPayloadBody::FileRef::sha256` is base16-lowercase.
- [ ] Existing `body` accessors gain a `body_bytes(&self) -> Option<Bytes>`
      that resolves the FileRef via the SandboxClient when called
      (used by Invalidation Detector — story 1.11 — and Verifier
      Worker context builder — story 1.9).
- [ ] Phase 0 DEBT #21 entry struck through with date + commit ref.
- [ ] Tests:
      - `small_payload_stays_inline` — 1 KB body returns `Inline`.
      - `large_payload_writes_to_eventfiles` — 100 KB body writes a
        `.bin` file and returns `FileRef`.
      - `extension_derived_from_content_type` — table coverage.
      - `body_bytes_round_trips_fileref` — write a payload, read it
        back via the accessor.
      - `eventfile_path_uses_event_id` — collision-free across two
        emits in the same session.
      - `removal_of_inline_preview_path` — grep-based test asserts the
        old `inline_preview_fallback` (or whatever the Phase 0 name
        was) is absent from the codebase.

## Non-goals

- Cleanup / TTL for `/workspace/.eventfiles/` — tied to Phase 0 DEBT
  #16 (workspace TTL).
- Streaming reads for huge files (>50 MB) — Phase 4+ if it ever bites.
- Compression of stored bodies — out of scope.
- A new HTTP endpoint to fetch event bodies — Phase 0 `/v1/workspace/:session_id/*path`
  is already adequate.

## Implementation steps

### 1. Module

```
crates/seasoned-hand-core/src/events/truncation.rs
```

```rust
pub const INLINE_CAP_BYTES: usize = 16 * 1024;

pub fn extension_for(ct: &str) -> &'static str {
    match ct.split(';').next().unwrap_or(ct).trim() {
        "text/plain"       => "txt",
        "text/markdown"    => "md",
        "application/json" => "json",
        "image/png"        => "png",
        "image/jpeg"       => "jpg",
        _                  => "bin",
    }
}

pub async fn write_large_or_inline(
    sandbox: &SandboxClient,
    session_id: &str,
    event_id: u64,
    body: &[u8],
    content_type: &str,
) -> Result<EventPayloadBody, EventStoreError> {
    if body.len() <= INLINE_CAP_BYTES {
        return Ok(EventPayloadBody::Inline { bytes: Bytes::copy_from_slice(body) });
    }
    let ext = extension_for(content_type);
    let path = format!("/workspace/.eventfiles/{event_id}.{ext}");
    sandbox.write_workspace_file(session_id, &path, body).await?;
    let sha256 = format!("{:x}", sha2::Sha256::digest(body));
    Ok(EventPayloadBody::FileRef {
        path: path.into(),
        content_type: content_type.into(),
        sha256,
        size: body.len() as u64,
    })
}
```

### 2. EventPayloadBody type

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventPayloadBody {
    Inline { bytes: Bytes },
    FileRef { path: PathBuf, content_type: String, sha256: String, size: u64 },
}

impl EventPayloadBody {
    pub async fn body_bytes(&self, sandbox: &SandboxClient, session_id: &str)
        -> Result<Bytes, EventStoreError>
    {
        match self {
            Self::Inline { bytes } => Ok(bytes.clone()),
            Self::FileRef { path, .. } => {
                let v = sandbox.read_workspace_file(session_id, path.to_str().unwrap()).await?;
                Ok(Bytes::from(v))
            }
        }
    }
}
```

### 3. Hook integration

In `crates/seasoned-hand-core/src/dispatch/hooks/event_emitting.rs`,
replace the Phase 0 inline-preview branch:

```rust
// before:
// if body.len() > 16384 { body = format!("[truncated; first 200 bytes:] {}", &body[..200]); }

// after:
let payload_body = write_large_or_inline(
    &ctx.sandbox, &ctx.session_id, /* event_id */ next_id,
    &body, &content_type,
).await?;
```

The event row's `body` column now stores `serde_json::to_vec(&payload_body)`
(or the existing JSON shape — keep the schema). The Phase 0 schema
likely already supports JSON bodies; this story refines what JSON shape
is stored.

### 4. SandboxClient additions (if not from story 1.4)

`write_workspace_file(session_id, path, bytes)` and
`read_workspace_file(session_id, path) -> Vec<u8>` are needed. Story 1.4
already added these; if not, add minimal implementations here. They
post to AIO Sandbox's existing `/v1/file/write` and `/v1/file/read`
endpoints.

### 5. DEBT close

Strike through Phase 0 DEBT #21 with date + commit ref.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core events::truncation::
cargo test -p seasoned-hand-core dispatch::hooks::event_emitting::tests
./scripts/spec-check.sh
grep -r 'inline_preview_fallback' crates/  # expect zero hits
```

Live: with the server up, run a task whose tool output exceeds 16 KB
(e.g. `info_search_web` on a verbose query). The event stream row
should carry a `file_ref` body; `ls /workspace/.eventfiles/` shows the
file.

---

## Files changed

- `crates/seasoned-hand-core/src/events/truncation.rs` (new)
- `crates/seasoned-hand-core/src/events/payload.rs` (modify —
  `EventPayloadBody` enum, `body_bytes` accessor)
- `crates/seasoned-hand-core/src/dispatch/hooks/event_emitting.rs`
  (modify — call helper, delete inline-preview branch)
- `crates/seasoned-hand-core/src/sandbox/client.rs` (modify if missing
  — `write_workspace_file` / `read_workspace_file`)
- `crates/seasoned-hand-core/src/events/tests.rs` (modify — 6 new tests)
- `specs/phase-0/DEBT.md` (close #21)

---

## Spec references

- `/specs/phase-1/architecture.md` §6 (pay-down statement), §7 (heap
  budget — file-ref keeps event-row size bounded).
- `/specs/phase-0/DEBT.md` #21 (origin).

---

## Commit message

```
fix(phase-1): story 1.14 - hook output-truncation file-ref path (DEBT #21)

- events::truncation::write_large_or_inline writes >16KB payloads to
  /workspace/.eventfiles/<event_id>.<ext> (extension derived from
  content-type) and returns EventPayloadBody::FileRef{path,
  content_type, sha256, size}; ≤16KB stays Inline{bytes}
- EventPayloadBody::body_bytes(&sandbox, &session_id) accessor resolves
  FileRef on read for downstream consumers (Invalidation Detector,
  Verifier context builder)
- EventEmittingHook (story 0.10) now routes through the helper; Phase 0
  inline-preview fallback removed wholesale
- 6 unit tests; grep-based check asserts the old code path is gone

Closes Phase 0 DEBT #21.

refs: /specs/phase-1/stories/story-1.14.md
```

---

## Notes for next story (1.15)

The hook chain can now emit arbitrarily-large outputs without inline
truncation. Story 1.15 (Narrator Hook) emits Message events whose body
is usually small (50 tokens) — no impact. Story 1.16 (3-track Browser
screenshots) routinely produces 200-400 KB PNGs — this story is the
prerequisite that makes Track C feasible.
