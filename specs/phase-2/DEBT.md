# Phase 2 — Technical Debt Ledger

> Append-only list of shortcuts, stubs, simplifications, and deferred
> work introduced during Phase 2. Same discipline as Phase 0 / Phase 1
> DEBT.md.
>
> Seeded at architecture phase boundary (2026-05-13). Items added during
> story implementation get appended below the seed block.

---

## Seed (from architecture v2.1, 2026-05-13)

### 1. WebhookChannel SSRF protection is permissive by default
- **Origin**: architecture.md §9 "Webhook delivery URL"
- **Severity**: **Medium**
- **What**: `WebhookChannel`'s `DeliverySink` impl rejects URLs that
  resolve to private / link-local / loopback addresses by default —
  but an operator allow-list bypasses the check. In Phase 2
  single-user, the operator IS the only caller, so trust is high.
- **Why**: Phase 5 multi-user makes webhook URLs user-supplied;
  attacker-controlled `reply_target.url` pointing at `http://10.0.0.1/admin`
  becomes a real SSRF.
- **Pay down**: Phase 5 tightens — webhook URLs from untrusted users
  must always resolve to public IPs; allow-list bypass requires admin
  scope.

### 2. Sandbox-side renderer toolchain via startup-install
- **Origin**: architecture.md §2.3 + §5 "Sandbox-side renderer toolchain"
- **Severity**: **Low**
- **What**: Phase 2 installs Pandoc + python-pptx + openpyxl via
  `apt install -y pandoc texlive-xetex && pip install python-pptx openpyxl`
  at session-create time (~30-60 s per session). Each new sandbox
  re-installs from scratch.
- **Why**: Avoids the operational lift of forking + publishing a
  `seasoned-hand-sandbox` image in Phase 2.
- **Pay down**: Phase 4 — once the renderer set stabilizes, bake a
  pre-published `seasoned-hand-sandbox:phase-4` image with the
  toolchain. Cuts session-spawn time from 30-60 s to <5 s.

### 3. Code-as-deliverable is git-tree-only in Phase 2
- **Origin**: architecture.md §2.3 + §12 q7
- **Severity**: **Low**
- **What**: Phase 2's "code" deliverable is the sandbox git tree
  itself (operator can `git clone` post-completion). No GitHub PR
  creation, no live deployment automation. The deliverable.format =
  "code" implies "go look at the sandbox workspace".
- **Why**: Auth-dependent GitHub/GitLab channels need Phase 5
  multi-user. Live deployment via `deploy_expose_port` exists as a
  Phase 0 tool but Phase 2 doesn't wire it into the deliverable flow.
- **Pay down**: Phase 4 — `GitHubChannel` (DeliverySink: PR creation)
  + composite `{ git_sha, deploy_url }` format.

### 4. Email allow-list is operator-curated
- **Origin**: architecture.md §9 "Email intake authentication"
- **Severity**: **Low**
- **What**: `INTAKE_EMAIL_ALLOWED_SENDERS` env (regex allow-list)
  defaults to empty (deny all). Operator manually whitelists own email
  + collaborators. No discovery / invite UX in Phase 2.
- **Pay down**: Phase 5 multi-user — per-user allow-list managed via
  account settings UI.

### 5. Provenance manifest size budget = 100 KB inline
- **Origin**: architecture.md §12 q5
- **Severity**: **Low**
- **What**: Provenance manifests stored inline in
  `deliverables.provenance_manifest` (JSON TEXT column). Manifests
  exceeding 100 KB (extreme long-running tasks with thousands of
  events) spill to `/workspace/.provenance/<task_id>.json` and the
  column stores a file-ref instead.
- **Pay down**: Phase 3+ — Curator may compress old manifests; Phase 5
  may move to a dedicated provenance store.

### 6. Skill / playbook tables empty in Phase 2
- **Origin**: architecture.md §2.12 + V009
- **Severity**: **n/a** (informational)
- **What**: V009 creates `skills` + `playbooks` tables with the
  expected schema; Phase 2 logic never writes rows. Phase 3 (learning)
  populates them.
- **Why**: Reserves the slot so Phase 3 is purely logic, not
  schema migration. Forward-compat principle.
- **Pay down**: Phase 3 — implement Curator + post-task playbook
  extraction.

### 7. Verifier rollback default still opt-in (Phase 1 DEBT #3 carryover)
- **Origin**: Phase 1 DEBT #3, Phase 2 closeout decision
- **Severity**: **Medium**
- **What**: Phase 2 closeout (story 2.27) collects verifier verdict
  precision from real "Do this overnight" runs and decides whether to
  flip `checkpoint_rollback_on_verifier_fail` default from `false` to
  `true`. If precision >90%, flip; else carry into Phase 3.
- **Pay down**: This story (2.27) — data-driven decision.

### 8. CLI auth deferred to Phase 5
- **Origin**: architecture.md §9 "CLI security"
- **Severity**: **Low**
- **What**: Phase 2 CLI is unauthenticated, talks to localhost. No
  `seasoned-hand auth login` flow. Operator runs CLI on the same
  machine as the server.
- **Pay down**: Phase 5 multi-user — OAuth/JWT, per-user tokens
  managed via `~/.seasoned-hand/credentials`.

### 9. Phase 1 DEBT items NOT paid down by Phase 2
- **Origin**: architecture.md §0 + Phase 1 closeout retrospective
- **Severity**: **n/a** (informational)
- **What**: Phase 1 DEBT items intentionally NOT addressed in Phase 2:
  - **#1, #2** ARCHITECTURE.md text drift — doc-only, no urgency
  - **#3** Verifier rollback default — addressed (see item 7 above)
  - **#4** Single invalidation heuristic — Phase 4 (Curator)
  - **#5** Single verifier slot for all triggers — Phase 4
  - **#6** Egress allowlist default — Phase 5 (multi-user)
  - **#7** Diversity Injector variants Rust-const — Phase 4 (Curator
    can promote variants to DB)
  - **#8** PostBrowserAction screenshot retention — folded into
    Phase 0 DEBT #16 (workspace TTL) which Phase 2 pays down
  - **#10** Lazy evidence_event_ids resolution — Phase 5
  - **#11** Sandbox git identity hardcoded — Phase 5
  - **#12** Verifier 5xx fail-closed — revisit if real outage data
- **Pay down**: Each linked to its target phase above.

---

## Story-introduced (chronological)

### 10. `Deliverable` struct lives inside `channel/delivery.rs`
- **Origin**: story 2.4 (`93fff98`), `crates/seasoned-hand-core/src/channel/delivery.rs`
- **Severity**: **Low**
- **What**: The `Deliverable` placeholder struct (V007 column shape)
  lives in `channel::delivery` instead of its eventual home in a
  dedicated `deliverable` module. Needed there so the
  `DeliverySink::deliver(target, deliverable)` trait signature is
  self-contained in story 2.4 (before 2.3 lands V007 +
  `DeliverableStore`).
- **Why**: Avoids forward-declaration dance; keeps 2.4 as a pure
  trait-surface story without dragging in V007 migration. Also avoids
  defining the Deliverable type in two places (channel module + 2.3's
  store module) and having to reconcile.
- **Pay down**: Story 2.3 — when `DeliverableStore` lands, decide
  whether to (a) keep `channel::Deliverable` as the canonical shape
  and have the store wrap it, or (b) move it into a top-level
  `deliverable` module and have `channel::delivery` re-export. Either
  way, the existing `Deliverable` shape is the V007 column projection,
  so no breaking change to callers.

---

## Categories quick-reference (same as Phase 0 / Phase 1)

| Severity | Meaning |
|---|---|
| **H** | Blocks the next phase's goals if not addressed |
| **M** | Will bite at scale or in a year, manageable today |
| **L** | Documentation / minor friction / one-line fix later |
