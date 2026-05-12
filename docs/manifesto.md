# Manifesto

> Why Seasoned Hand exists.

---

## The two breakthroughs of 2025

**Manus** proved AI can finish a task — not chat about it, not draft a snippet, but actually complete deep work. Fifty tool calls. Hours of autonomous browsing, scripting, verifying. A landing page. A market analysis. A spreadsheet that didn't exist when you went to bed.

**Hermes Agent** proved AI can remember — not just facts, but skills. Complete a hard task once, and the agent writes a playbook so it gets faster next time. Time becomes the agent's friend rather than its eraser.

These breakthroughs sit on different axes. Manus is depth (within a single task). Hermes is time (across many tasks). Both are real. Neither is enough alone.

---

## The empty quadrant

```
            Depth (autonomous execution)
                  ↑
          high  │ Manus           [ Seasoned Hand ]
                │
                │                 Hermes
                │
          low   │ ChatGPT         Notion AI
                └─────────────────→
                 single session   accumulating over time
```

No one is in the top-right yet. That's the space for an autonomous AI that **finishes hard work** *and* **gets better at it over time**. Not an assistant. Not a copilot. An **employee**.

---

## What "employee" means

An employee is hired, briefed, and trusted to deliver. The manager doesn't watch every keystroke. The manager checks the result, gives feedback, and the employee learns what the company expects.

Specifically, a good employee:

1. **Finishes what you delegate** — autonomous task completion
2. **Brings back deliverables** — files, reports, data — not just words
3. **Asks when blocked** — but only when blocked
4. **Doesn't repeat mistakes** — learns from verified work
5. **Knows the company's way** — accumulates organizational context

Manus does 1, 2, 3. Hermes gives us 4 and 5. We need both. We need them in the same system.

---

## Why open source

Three reasons.

**Trust**. Autonomous agents make consequential decisions — sending emails, editing code, calling APIs that cost money. A black-box SaaS asks you to trust the vendor. Open source asks you to verify.

**Sovereignty**. The interesting questions about AI agents involve your data — your tasks, your customers, your code, your knowledge. Sending all of it to one vendor concentrates risk we shouldn't accept lightly. Self-hosting is a way out.

**Composition**. The agent space is moving fast. New models, new tools, new patterns. A closed system locks you to one vendor's pace. An open one lets you mix the best of what's available and swap parts as the field evolves.

---

## What we are not building

- A chatbot. (ChatGPT is fine for that.)
- A coding copilot. (Cursor, Claude Code, Codex, and similar tools are excellent.)
- A platform for "AI agents to talk to each other." (Most multi-agent demos are theater.)
- A no-code workflow builder. (Different problem, different tool.)
- A SaaS we monetize. (See: open source.)

We're building one thing: an autonomous employee that gets seasoned by the work you give it.

---

## What "seasoned" means

In English, *to season* is what time does to wood — drying it, hardening it, making it useful for fine work. A *seasoned hand* is someone whose skill came from doing the work, not reading about it. The English idiom doesn't translate cleanly; the closest Korean equivalent is 길든 — *broken in by use*.

This is the right metaphor for what we're building. Not "intelligent" (overpromising), not "smart" (cheap), not "advanced" (meaningless). Seasoned. The agent has done the work, and the work has shaped it.

---

## The deal we propose

You give Seasoned Hand a task.
It works on the task — deeply, autonomously, until done.
When it succeeds, it writes down what worked.
Next time you give it a similar task, it's faster.
Over months, it learns how you work, what your standards are, what your context is.

That's the deal. Hire the hand. Watch it grow seasoned.

---

## Honest about what's hard

We're not claiming this is solved. The agent will fail. It will produce wrong work. It will need supervision — especially early. We're not promising magic; we're promising a system you can audit, host yourself, improve, and shape to your work.

The way past these failure modes is not better marketing. It's:
- Specification-driven design (so we can verify behavior matches intent)
- Verification gates (so failures get caught before they propagate)
- Conservative learning (only from verified successes, not noisy interactions)
- Accountability trails (so when something goes wrong, you can find out why)

We build all four, openly.

---

## How to read this project

- Want to **use it**? Start with [`README.md`](../README.md).
- Want to **understand the design**? Read [`/specs/01-architecture/ARCHITECTURE.md`](../specs/01-architecture/ARCHITECTURE.md).
- Want to **contribute**? Read [`CONTRIBUTING.md`](../CONTRIBUTING.md).
- Want to **understand the methodology**? Read [`/docs/methodology.md`](methodology.md).
- Want the **brand assets**? See [`/docs/brand.md`](brand.md).

---

*Every task makes the hand wiser.*
