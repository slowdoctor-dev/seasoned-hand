# PRINCIPLES

> Principles guide decisions. When a choice is unclear, refer here.

---

## 1. Specs are the source of truth

Code is derived from specs, not the other way around. When code and spec
diverge, the spec wins until reconciled. Update spec in the same PR.

**Why:** Without this, AI-generated code drifts. The spec is the only stable
anchor across many short AI sessions.

## 2. One tool per agent loop iteration

The agent picks ONE tool, calls it, observes, decides next. Never bulk
operations. Never parallel tool calls within a single iteration.

**Why:** Manus principle, validated empirically. Parallel calls cause cascade
failures. Sequential is slower per iteration but converges reliably.

## 3. Append-only event stream

Events never UPDATE or DELETE. New observations create new events.

**Why:** KV-cache friendly (context reuse across iterations), full audit trail,
recoverable from any point.

## 4. Conservative learning

The system only learns from **verified** successful work. Not from
conversations. Not from drafts. Not from one-off experiments.

**Why:** Bad patterns reinforce silently. A verified-only policy means
fewer skills, but every skill is real.

## 5. Fresh context per story

Every implementation story starts with a clean AI session. No carryover.
No "continue where we left off."

**Why:** AI context degrades. Stories reload the spec from scratch. The spec
acts as memory; the AI doesn't need to.

## 6. Verification before completion

Every story has explicit acceptance criteria. Every commit passes lint, type
check, tests, and spec compliance. Without exception.

**Why:** Without automated gates, AI-assisted work erodes quality. Gates are
the only scalable defense.

## 7. LLM-agnostic from day one

Production code must work whether the developer uses Claude Code, Codex CLI,
Cursor, or anything else. AGENTS.md is universal source of truth; tool-
specific files (CLAUDE.md, .codex/) only contain tool-specific extensions.

**Why:** The AI tool space moves faster than this project. Tool lock-in
becomes a liability within 12 months.

## 8. Domain-neutral core

The core platform makes no domain assumptions (medical, legal, finance, etc.).
Domain extensions go in user-space (playbooks, SOPs, glossary entries).

**Why:** Domain assumptions in core block the project from one vertical to
another. Specific domains are user-extensions.

## 9. Explicit over implicit

Configuration is explicit. Magic is bad. If the system does something the
user didn't ask for, that's a bug.

**Why:** Autonomous systems need explainable behavior. Implicit defaults
hide accountability.

## 10. Failure-tolerant, never failure-hiding

The system surfaces failures (verifier flags, circuit breakers, error events
preserved in stream). Never silently retries to mask a real problem.

**Why:** Hidden failures compound. Visible failures get fixed.

## 11. Trust through audit trail

Every decision, tool call, and outcome is recorded. The user can always
answer "why did the agent do that?"

**Why:** Trust scales with auditability. Magic does not scale.

## 12. Quiet competence over loud marketing

In docs, in commits, in error messages: plain language. No hype words. No
"revolutionary." No "AI-powered." We say what is, not what sounds good.

**Why:** The world is full of AI marketing. We sound different by being
plainer than competitors.

## 13. Build for the next session, not the current one

When writing code or specs, ask: "Will a fresh AI session, 6 months from
now, understand this?" If not, add context.

**Why:** This project will outlive any single AI session. The spec is the
intergenerational memory.

## 14. Bilingual but English-first for ecosystem reach

Code, comments, and primary specs in English. Korean docs welcomed. Both
languages in this repo are first-class for human readers; English is
required for AI tool compatibility and global community reach.

**Why:** Code in non-Latin scripts breaks tools silently. Docs in two
languages are richer than one.

## 15. Time is the agent's friend

The longer a user uses the system, the better it should perform on their
work. This is the test of whether "learning" is real.

**Why:** Without this property, we're just another agent framework.
With it, we're an employee.

## 16. Context is RAM, sandbox filesystem is disk

Important state lives in files within the sandbox, not in the context
window. Re-reading a file gives 100% accuracy; relying on long-context
recall is lossy.

For long tasks (>20 iterations or >50K tokens):
- Save key findings, URLs, intermediate data to files
- Reference files by path in subsequent iterations
- Treat conversation summarization as lossy compression
- Treat file re-reads as the ground truth

**Why:** Manus calls this "External Brain" strategy and confirmed it
directly. Without it, long-task accuracy degrades regardless of context
window size. Validated against direct Manus Q&A.

## 17. Plans are sticky context anchors, never free text

The agent maintains a structured plan (goal + phases + current_phase_id)
that is injected at the top of every iteration's context. Plans are
NOT free text inside a message; they are first-class artifacts with
explicit update operations.

For every task:
- Plan is created at task start (planner slot, structured JSON output)
- Plan is read at the start of every iteration
- Plan advances explicitly (`plan_advance`) or rebuilds (`plan_update`)
- Plan survives context compression — auxiliary `compression` slot must
  preserve plan verbatim

**Why:** Without plan-as-PCB, agents suffer "goal drift" — getting so
focused on a sub-problem they forget the original goal. Manus uses the
same pattern and validates it as fundamental to long-task success.
See ADR-010.

---

## When in doubt

Choose the option that:

- Is more explicit (over implicit)
- Is more testable (over "we'll know")
- Is more reversible (over irreversible)
- Is more visible (over hidden)
- Trusts the spec more (over trusting the AI's latest output)

When two principles conflict, refer to BASELINE.md for hard decisions.
When still stuck, write a new ADR documenting the choice.

---

*Principles are deliberately fewer than rules. They're meant to be applied,
not consulted as a manual.*
