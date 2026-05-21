# Security Policy

## Reporting a vulnerability

If you discover a security vulnerability in Seasoned Hand, please report it privately.

**Do not open a public issue.**

### How to report

Use GitHub's [private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability) feature on this repository.

If that's not available, contact the project maintainer via the email listed on their GitHub profile.

### What to include

- Description of the vulnerability
- Steps to reproduce
- Affected versions
- Potential impact
- Suggested fix, if you have one

### Response timeline

- **Acknowledgement**: within 7 days
- **Initial assessment**: within 14 days
- **Fix or mitigation**: depends on severity
- **Public disclosure**: coordinated with you, typically after fix is released

## Scope

Security issues in scope:

- The Seasoned Hand control plane (Rust)
- The frontend (Next.js)
- The model router (12-slot routing logic)
- Tool dispatcher and sandbox integration
- Configuration handling (API keys, secrets)
- Hooks and verifier bypass

Out of scope:

- Issues in upstream dependencies (report to those projects)
- Issues in LLM providers themselves (Anthropic, OpenAI, etc.)
- Issues in Bifrost (report at maximhq/bifrost)
- Social engineering of users

## Disclosure

We follow coordinated disclosure. Vulnerabilities will be:

1. Fixed in a private branch
2. Released as a patch version
3. Published as a GitHub Security Advisory
4. Credited to the reporter (if they wish)

## Security model

Seasoned Hand is self-hosted. Each operator is responsible for their own:

- API key management
- Network isolation
- Access control
- Backup and recovery

We aim to provide:

- Sane defaults (e.g., sandbox isolation per task)
- Clear audit trails (event stream is append-only)
- Verification gates (verifier reviews work before declaring success)
- No surprising side effects (tools require explicit configuration)

We do not:

- Phone home
- Send telemetry without explicit opt-in
- Make network calls outside what the user configures

## Request authentication & network exposure (IMPORTANT)

The control plane derives caller identity — `tenant_id`, `organization_id`,
`actor_user_id`, and role — from plaintext `x-seasoned-hand-*` request headers. It
does **not** independently authenticate the caller (verifiable API-key / OAuth authn
is the planned Phase 6 auth decision). The deployment model is therefore:

- **Default bind is `127.0.0.1` (loopback).** Every sensitive HTTP/WebSocket handler
  additionally enforces a loopback check, so on the default bind only processes on the
  same host can reach them. This is the supported single-operator posture.
- **If you set `HOST` to a non-loopback address** (e.g. `0.0.0.0`), the server logs a
  startup `SECURITY:` warning. In that mode you **MUST** place a trusted reverse proxy /
  gateway in front that authenticates every caller and sets the `x-seasoned-hand-*`
  headers itself, and you must **not** expose the control-plane port directly. Without
  that gateway, any client that can reach the socket can assert
  `x-seasoned-hand-org-role: admin` for any tenant and gain full access.

Tenant isolation within the control plane (one tenant cannot read/write another's data
once identity is established) is enforced at every surface and covered by the
`phase5_cross_tenant_isolation_harness`; see `specs/phase-5/REVIEW.md`. That isolation
assumes the *identity itself* is trustworthy — which is the gateway's responsibility in
a multi-tenant deployment.

## Known security considerations

- The agent can execute shell commands inside the sandbox. The sandbox is the security boundary. Do not run on untrusted code paths or with mounted host filesystems.
- The agent can call LLM APIs that may charge money. Cost caps are enforced; verify they match your budget.
- The agent can browse the web. By default, no special filtering is applied. Consider configuring egress restrictions if needed.
- Skills and playbooks are user-extensible. Treat third-party playbooks with the same caution as third-party code.

---

Thank you for helping keep Seasoned Hand secure.
