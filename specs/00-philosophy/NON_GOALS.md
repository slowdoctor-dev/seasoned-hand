# NON_GOALS

> What this project explicitly is NOT doing. Scope is defined as much by
> exclusion as by inclusion. When tempted to add something, check here first.

---

## Not a chatbot

ChatGPT, Claude.ai, and similar already exist. We're not building another.

If you're using Seasoned Hand and find yourself having a long conversation
instead of getting a task done, something is wrong.

## Not a coding copilot

Cursor, Claude Code, GitHub Copilot, Codex CLI, Cline, Aider — these handle
"help me write code" excellently. Seasoned Hand uses them; it doesn't
compete with them.

## Not a no-code workflow builder

Zapier, n8n, Make.com, Activepieces — these are for "trigger A causes B."
Seasoned Hand is for "complete this task using whatever tools you need."
Different problem, different tool.

## Not a multi-agent orchestration platform

"AI agents talking to each other" is mostly theater in 2026. Most multi-
agent demos can be replaced by a single well-instructed agent.

Seasoned Hand uses sub-agents internally for context isolation, but does
not expose multi-agent orchestration as a feature.

## Not a SaaS

There will never be `seasonedhand.com/signup`. The project is self-hosted.

If commercial offerings emerge later (hosting, support, training), they
will be from third parties, not the project itself.

## Not a closed system

Closed system = vendor lock-in = death of trust. The license is Apache-2.0
for this reason (was MIT through Phase 5; see ADR-015). No "open core, paid
features." No "free for personal, paid for commercial." Just open.

## Not a chat memory store

ChatGPT has memory. Claude has memory. So do we, but for a different
purpose: learning what works for repeating tasks. We don't try to remember
"the user mentioned their dog's name last Tuesday."

User-detail memory is in scope only insofar as it helps task completion.

## Not a personality

We're not building "an AI with character." The agent should feel like a
competent professional employee, not a friend. Plain, helpful, focused.
No personality tics, no jokes, no emoji in production output.

## Not a multi-modal generator

No image generation. No audio generation. No video generation. The agent
can *use* models that do these (via the auxiliary slot), but generating
multimedia isn't the project's purpose.

## Not domain-specific

The core makes no assumptions about medical, legal, financial, or any
specific vertical. Users add domain context via playbooks and glossary,
not via core modifications.

If you find yourself adding "if medical_mode then..." to the core, stop.
That belongs in user-space.

## Not a fine-tuning platform

We don't train models. We use models. Fine-tuned models are in scope as
something users can plug in via the model router; the project itself
doesn't fine-tune.

## Not a marketplace

No "skill marketplace." No "playbook store." Users share playbooks via
git, the same as code. No central registry.

If a community emerges and wants a registry later, it can build one;
the project itself doesn't operate one.

## Not a mobile-first product

Mobile responsive — yes. Mobile-first — no. The primary interface is
desktop web (since autonomous agents need keyboards, multiple panels,
and long-form input).

A future mobile companion app is possible but explicitly not part of v1.

## Not a fast onboarding product

Self-hosted agents have inherent setup friction (Docker, API keys, config).
We document this clearly. We don't pretend to be a 30-second sign-up SaaS.

If a developer can't spend 30 minutes setting up, this isn't for them.
That's fine.

## Not for users who can't self-host

The product assumes you can run Docker, set environment variables, edit
YAML, read logs. If this is too much, use ChatGPT.

This isn't elitism — it's clarity about who the user is.

## Not a Korean-only product (despite Korean origins)

This project originated in Korea, by a Korean author. Docs may be bilingual.
But the product is global: English primary, Korean welcome, other languages
via translation contributions.

We're not building a "for Korea only" tool. We're not artificially limiting
the audience.

## Not an "AI assistant for everything"

We're an autonomous task-completing employee. Not a personal organizer.
Not a learning companion. Not a meditation guide. Not a fitness coach.

If you find yourself describing the project with words like "everything,"
"general-purpose AI," or "your AI for life" — stop. That's vague enough to
be meaningless.

---

## When tempted to add a feature

Ask:

1. Does this fit the "autonomous employee who gets seasoned by work" frame?
2. Is this in core, or could it live in user-space (playbook, glossary)?
3. Are we doing this because users asked, or because it seemed cool?
4. What does adding this prevent us from doing better?

If the answer to any of these is uncomfortable, the feature probably
belongs on this list, not in the codebase.

---

*Non-goals protect focus. Saying "we don't do that" is one of the most
valuable design decisions a project makes.*
