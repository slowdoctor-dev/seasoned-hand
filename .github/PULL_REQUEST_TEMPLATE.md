## Story

- Story file: `/specs/phase-N/stories/story-X.Y.md`
- Closes #(issue number)

## What changed

(Brief: what was implemented. Detailed plan was already in pre-implementation discussion.)

## Spec compliance

- [ ] Implementation matches the story's acceptance criteria exactly
- [ ] No scope expansion beyond the story
- [ ] If the spec needed updating, the update is included in this PR

## Verification gates

- [ ] `cargo clippy --all-targets -- -D warnings` passes (no warnings)
- [ ] `cargo fmt --check` passes
- [ ] `cargo test --workspace` passes
- [ ] `just check-ui` passes (if the Dioxus UI changed — fmt + clippy + wasm check)
- [ ] `./scripts/spec-check.sh` passes

Or simply: `just verify` passes.

## Story-specific verification

(Commands from the story's "Verification" section.)

```bash
# (paste commands and expected output)
```

## Files changed

- `path/to/file` — what
- `path/to/file` — what

## Story status update

- [ ] Story status in the file changed from `in-progress` to `done`
- [ ] "Notes from execution" section added if anything noteworthy

## Commit message

(Use the exact commit message from the story file.)

---

## For reviewer

Checklist (see `CONTRIBUTING.md`):

- [ ] Maps to exactly one story
- [ ] All acceptance criteria met
- [ ] CI green
- [ ] Spec matches code
- [ ] No expanded scope
- [ ] Commit message follows convention
- [ ] No TODOs without linked issues
- [ ] Story status updated
