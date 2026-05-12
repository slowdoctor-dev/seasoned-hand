# VISION

> What this project is for, in detail.

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
                └──────────────────────→
                 single session   accumulating over time
```

Manus proved AI can finish hard tasks. Hermes proved AI can remember work.
Both are real breakthroughs. Neither is complete alone.

No system yet occupies the top-right: deep autonomous execution **and**
accumulated learning. That's what Seasoned Hand is for.

## What we're building

A digital **employee**, not a chatbot.

An employee is hired, briefed, and trusted to deliver. The manager doesn't
watch every keystroke. The manager checks the result, gives feedback, and
the employee learns what the manager expects.

The five qualities of a good employee:

1. **Finishes what you delegate** — autonomous task completion (Manus does this)
2. **Brings back deliverables** — files, reports, data, not just words (Manus)
3. **Asks when blocked** — but only when truly blocked (Manus)
4. **Doesn't repeat mistakes** — learns from verified work (Hermes does this)
5. **Knows the organization's way** — accumulates context over time (Hermes)

We need all five in one system.

## What "seasoned" means

In English, *to season* is what time does to wood — drying it, hardening it,
making it useful for fine work. A *seasoned hand* is someone whose skill came
from doing the work, not reading about it.

The Korean equivalent is **길든** — *broken in by use*. Not "intelligent"
(overpromising), not "smart" (cheap), not "advanced" (meaningless). Seasoned.
The agent has done the work, and the work has shaped it.

## Why open source

Three reasons.

**Trust.** Autonomous agents make consequential decisions — sending emails,
editing code, calling APIs that cost money. A black-box SaaS asks you to
trust the vendor. Open source asks you to verify.

**Sovereignty.** The interesting questions about AI agents involve your
data — your tasks, your customers, your code, your knowledge. Sending all
of it to one vendor concentrates risk we shouldn't accept lightly.
Self-hosting is a way out.

**Composition.** The agent space is moving fast. New models, new tools,
new patterns. A closed system locks you to one vendor's pace. An open one
lets you mix the best of what's available and swap parts as the field
evolves.

## Why now

The 2026 inflection point. Three things converged:

1. **Models got good enough** for agentic loops (50+ tool calls without collapse)
2. **Tool ecosystems matured** (MCP standardized, browser tools robust)
3. **Self-hosting infrastructure** got cheap ($5 VPS, NAS, single-binary deploys)

This wasn't possible in 2024. Now it is.

## What success looks like

In 12 months:
- Anyone can `docker compose up` and have a working autonomous agent
- Any LLM works (cloud or local)
- After a month of use, the agent is meaningfully faster at the user's tasks
- The user trusts it enough to delegate without watching

In 24 months:
- Active contributor community
- Multiple deployment patterns (solo dev, small org, self-hosted enterprise)
- Documented case studies (without spam vendor pitches)
- Recognized as the open alternative to closed agentic SaaS

## What this is not

Repeat: **this is not**:

- A chatbot (ChatGPT does that)
- A coding copilot (Cursor, Claude Code, Codex do that)
- A platform for "AI agents talking to each other" (mostly theater)
- A no-code workflow builder (Zapier/n8n already exist)
- A SaaS to monetize

One thing only: an autonomous employee that gets seasoned by your work.

## How we'll know we're wrong

If after 6 months:

- No one outside the original author has run it
- The "learning" produces playbooks that nobody finds useful
- Single-session execution is brittle (verifier fails > 30% of time)
- Self-hosting is too painful (setup takes > 1 hour for a developer)

Then we revisit the premise. The vision is testable.

## How we'll know we're right

If after 12 months:

- GitHub stars > 5,000 (rough market validation)
- 100+ self-hosted deployments (reported via opt-in telemetry)
- Average user reports task completion time decreasing over 3 months
- External contributors merging non-trivial PRs

Then the bet was correct.

---

*This document captures the why. For the how, see `ARCHITECTURE.md`.
For the when, see `ROADMAP.md`. For the what right now, see the current
phase folder.*
