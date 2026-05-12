# ADR-004: AIO Sandbox (Docker) per session

Status: Accepted
Date: 2026

## Context

The agent executes shell commands, browser actions, file operations, and
arbitrary code. Sandboxing is a hard requirement for self-hosted deployment.

Three patterns considered:

1. Local execution (no sandbox) — fastest, dangerous
2. Containerized sandbox per session (Docker) — isolated, slower setup
3. Lightweight VM (Firecracker, like Manus) — best isolation, complex ops

## Decision

Use **AIO Sandbox** (a pre-built container with Ubuntu + Chromium + tmux +
VNC/noVNC + ttyd) via the `bollard` Rust Docker SDK. One sandbox per
agent session.

Sandbox lifecycle:
- Created on session start
- Paused on session idle (Docker pause API)
- Destroyed on session end
- Workspace mounted to host (persistent across pause/resume)

## Consequences

**Positive:**
- Filesystem isolation: agent can't touch host files outside workspace
- Network isolation: configurable egress
- Repeatable: any user gets the same environment
- noVNC + ttyd give browser/terminal visibility to the UI

**Negative:**
- Container startup adds ~3 seconds to session cold start
- Disk usage: each workspace persists (cleanup job needed)
- Resource limits: large concurrent sessions may exhaust host

**Neutral:**
- Docker requirement for self-hosting (most developers have this anyway)

## Alternatives considered

### Alternative A: Firecracker microVM (Manus's choice)
Strongest isolation. Used by Manus and AWS Lambda. But:
- Complex ops (Linux KVM, jailer setup)
- Not portable to macOS dev machines
- Overkill for solo and small-team self-hosting

Rejected on operational complexity. May revisit for enterprise tier.

### Alternative B: Local execution with restricted shell
Fastest. Simplest. But:
- One bad tool call destroys the host
- Real risk: AI-driven `rm -rf /` has happened (community-reported, 2025)

Rejected on safety grounds.

### Alternative C: Process-level isolation (bubblewrap, firejail)
Linux-only. Better than nothing. But:
- Not cross-platform
- Weaker than container isolation
- Less observable from the UI

Rejected on portability.

## References

- AIO Sandbox: agent-infra/sandbox
- bollard (Rust Docker SDK)
- Manus's microVM choice analyzed in our research docs
