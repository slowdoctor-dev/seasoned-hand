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

## Known security considerations

- The agent can execute shell commands inside the sandbox. The sandbox is the security boundary. Do not run on untrusted code paths or with mounted host filesystems.
- The agent can call LLM APIs that may charge money. Cost caps are enforced; verify they match your budget.
- The agent can browse the web. By default, no special filtering is applied. Consider configuring egress restrictions if needed.
- Skills and playbooks are user-extensible. Treat third-party playbooks with the same caution as third-party code.

---

Thank you for helping keep Seasoned Hand secure.
