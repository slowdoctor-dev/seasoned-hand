# Story 2.7 — Brief shape + DeliverableSpec typed schema

> **Status**: ready
> **Estimated**: 1.5 hours
> **Dependencies**: 2.2
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.2

---

## Goal

Land the typed `Brief` + `DeliverableSpec` shapes that the Initializer
will emit (story 2.8) and the LLM tools will consume (story 2.14).
Includes JSON-schema validation with explicit caps (20 phases, 50
success criteria, 20 deliverables). No runtime behavior — just the
data shape + parser + validator.

## Acceptance criteria

- [ ] `seasoned_hand_core::project::Brief` (in `project/brief.rs`):
      ```rust
      pub struct Brief {
          pub goal: String,
          pub phases: Vec<BriefPhase>,
          pub success_criteria: Vec<String>,
          pub expected_deliverables: Vec<DeliverableSpec>,
      }
      pub struct BriefPhase {
          pub id: u32,
          pub title: String,
          pub capabilities: Vec<String>,
      }
      pub struct DeliverableSpec {
          pub filename: String,
          pub format: DeliverableFormat,
          pub description: Option<String>,
      }
      pub enum DeliverableFormat {
          Markdown, Json, Csv, Docx, Pdf, Html, Pptx, Xlsx, Code, Url,
      }
      ```
- [ ] `DeliverableFormat::from_filename(&str) -> Option<Self>` infers
      from extension (`.md` → Markdown, `.docx` → Docx, etc.). `.txt` →
      Markdown alias. Unknown extension → `None`.
- [ ] `Brief::validate() -> Result<(), BriefError>` enforces caps:
      `phases.len() <= 20`, `success_criteria.len() <= 50`,
      `expected_deliverables.len() <= 20`. Per-string length caps:
      `goal.len() <= 4000`, `success_criteria[i].len() <= 200`,
      `phase.title.len() <= 200`. Returns typed errors per cap.
- [ ] `Brief::from_planner_output(raw: &str) -> Result<Self, BriefError>`
      parses the planner-slot LLM's response. Accepts JSON or
      markdown-with-JSON-fenced-block (use the Phase 1 planner-parse
      pattern from story 1.4). On parse failure, returns
      `BriefError::ParseFailed { reason }`.
- [ ] `Brief::serialize() -> serde_json::Value` for storage in
      `tasks.brief` (per V006).
- [ ] Unit tests:
      - `brief_serialize_round_trips`
      - `deliverable_format_inferred_from_filename` (table-driven across
        all 8 mapped extensions + 2 unknowns)
      - `brief_validate_rejects_too_many_phases`
      - `brief_validate_rejects_too_long_goal`
      - `brief_parses_fenced_json` (the markdown-with-fenced-block case)
      - `brief_parses_naked_json`

## Non-goals

- Initializer integration (story 2.8 wires this in)
- HTTP routes that take a Brief (the existing `Initializer` callers
  flow it through unchanged)
- LLM call to produce the Brief (already exists in story 1.4's
  Initializer; 2.7 just types the output)

---

## Implementation steps

### 1. Brief types

```
crates/seasoned-hand-core/src/project/brief.rs
```

Use serde derive (`Serialize`, `Deserialize`). `DeliverableFormat`
serde-renames to lowercase strings matching the architecture spec's
type union.

### 2. Validation + caps

`BriefError` enum (thiserror): `ParseFailed`, `TooManyPhases`,
`TooManySuccessCriteria`, `TooManyDeliverables`, `GoalTooLong`,
`PhaseTitleTooLong`, `CriterionTooLong`, `DeliverableFilenameTooLong`,
`UnknownFormat(String)`.

### 3. Parser

`Brief::from_planner_output` — extracts JSON from `\`\`\`json ... \`\`\``
fenced block if present, else parses naked. Mirrors Phase 1 1.4's
planner parser. Reuse the parser if it's already factored out.

### 4. Tests

In-module `tests.rs` with the listed table-driven tests.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core project::brief
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/project/brief.rs` (new)
- `crates/seasoned-hand-core/src/project/mod.rs` (modify — re-export)
- `crates/seasoned-hand-core/src/project/tests.rs` (modify — append
  brief tests)

---

## Spec references

- `/specs/phase-2/architecture.md` §2.2 (Brief shape), §2.3
  (DeliverableSpec format enum + filename mapping)

---

## Commit message

```
feat(phase-2): story 2.7 - Brief shape + DeliverableSpec typed schema

- Brief { goal, phases, success_criteria, expected_deliverables } typed
  struct. Serde-derived. Stored in tasks.brief (V006).
- DeliverableSpec { filename, format, description }. DeliverableFormat
  enum maps 1-1 to architecture §2.3 format list.
- DeliverableFormat::from_filename infers from extension.
- Brief::validate enforces caps (20 phases / 50 criteria / 20
  deliverables) + per-string length caps. Typed BriefError variants.
- Brief::from_planner_output parses planner-slot LLM responses (naked
  JSON or markdown-fenced JSON, mirroring Phase 1 1.4's pattern).
- 6 unit tests.

refs: /specs/phase-2/stories/story-2.7.md
```

---

## Notes for next story (2.8)

Brief shape is in. 2.8 extends the Initializer with
`run_with_confirmation`: emit Briefing event → await WS user_response
or 5-min auto-confirm → seed Plan from Brief.
