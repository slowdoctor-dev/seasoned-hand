# Contributing to Seasoned Hand

Thanks for considering a contribution.

This project follows **Spec-Driven Development**. Code is derived from specs, not the other way around. Read `/AGENTS.md` and `/docs/methodology.md` before contributing.

---

## How contributions are organized

Work is broken into **stories** — 1-3 hour units in `/specs/phase-N/stories/story-N.X.md`. Each story:

- Has explicit acceptance criteria
- Maps to exactly one PR
- Maps to exactly one commit
- Is independently mergeable

Pick a story from the current phase that's not yet `Status: in-progress` or `done`.

## Workflow

1. **Pick a story** from `/specs/phase-N/stories/`. Verify status is `ready`.
2. **Comment on the corresponding issue** (or open one) to claim it. This prevents duplicate work.
3. **Update story status** to `in-progress` in your branch.
4. **Start a fresh AI session** (Claude Code or Codex) with `/prompts/gsd-execute-story.md`.
5. **Discuss → Plan → Execute → Verify** (see workflow in prompt).
6. **Verify all gates pass**: `just verify`.
7. **Open PR** with the commit message from the story file.
8. **Wait for review**. Reviewer checks spec compliance, not just code.
9. **After merge**: update story status to `done`.

## Per-story rules

- **One story per PR.** No bundling.
- **No scope expansion.** If the story is too small or large, comment and propose a split, don't silently expand.
- **Spec must match code.** If implementation requires divergence, update the spec in the same PR.
- **Fresh AI context** per story. Don't carry context across stories.
- **All verification gates** must pass before requesting review.

## Code style

See `/AGENTS.md` § 7.

- Rust: edition 2024, zero clippy warnings, `thiserror` for errors
- TypeScript: strict mode, pnpm, functional components, no `any`
- Markdown: ATX headers, code blocks with language tags, wrap at 100 chars

## Commit messages

```
feat(phase-N): story X.Y - brief description

- what changed
- why

refs: /specs/phase-N/stories/story-X.Y.md
```

Types: `feat`, `fix`, `docs`, `chore`, `refactor`, `test`, `spec`.

## Spec changes

`/specs/01-architecture/ARCHITECTURE.md` is the **immutable** top-level architecture. Changes require:

1. Issue describing the change and rationale
2. Discussion before PR
3. Version bump in the spec
4. Migration plan if breaking

`/specs/phase-N/*.md` are mutable within their phase. Updates allowed in same PR as code that requires them.

## Reviewing a PR

Checklist:

- [ ] Maps to exactly one story
- [ ] Story acceptance criteria all met
- [ ] All verification gates pass in CI
- [ ] Spec matches code (no silent divergence)
- [ ] No expanded scope beyond the story
- [ ] Commit message follows convention
- [ ] No TODOs without linked issues
- [ ] Story status updated to `done`

## Discussions, ideas, large changes

Open a GitHub Discussion (when available) or an issue with the `discussion` label. Don't open a PR for changes that haven't been discussed.

## Code of Conduct

By participating, you agree to abide by the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

By contributing, you agree your contributions will be licensed under the project's [MIT License](LICENSE).

---

## A note on AI-generated contributions

This project is built with AI assistance. We expect contributors to use Claude Code, Codex, Cursor, or similar. That's fine.

What we require:

- You read the spec before generating code
- You verify the output matches the spec
- You take responsibility for the contribution as if you wrote it by hand
- You don't submit code you don't understand

The bar isn't "did a human type this?" The bar is "is this correct, intentional, and matches the spec?"

---

Thanks for helping the hand get more seasoned.
