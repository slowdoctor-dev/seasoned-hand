# ADR-016: Replace Next.js frontend with Dioxus (unified-Rust UI for multi-platform)

Status: Accepted
Date: 2026-06-05
Amends: the frontend clause of [ADR-002](ADR-002-rust-typescript-hybrid.md)
        (the "Rust backend" clause of ADR-002 stands unchanged)

## Context

[ADR-002](ADR-002-rust-typescript-hybrid.md) chose **Rust backend + TypeScript /
Next.js frontend** and, in its "Alternatives considered → Alternative C: All-Rust
(Tauri-style desktop app)", explicitly rejected an all-Rust UI on four grounds:

1. React ecosystem unavailable
2. Frontend dev velocity drops significantly
3. AI tool support for Rust UIs is weak
4. Single-binary deployment vs the standard browser experience

That call was correct **at the start of the project** — the UI surface was an
unknown moving target and Rust UI tooling was immature. The situation has shifted
now that Phases 0–5 have shipped, and a new requirement has appeared:

- **New goal: multi-platform reach.** The product needs to run on web **and**
  desktop **and** mobile. The "Beyond v1" roadmap item (a read-only mobile
  companion) is a partial answer that would mean maintaining *two* frontends.
- **The UI surface is now known and stable.** It is a defined spec — a 3-panel
  operator console (`TaskList | Chat | AgentComputer`) plus briefing/approval
  flows — not a greenfield target. Porting a settled design carries far less
  velocity risk than ADR-002's premise (2) assumed.
- **Dioxus ≠ raw Tauri.** Dioxus gives a React-like authoring model in Rust —
  RSX (JSX-alike), components, and signal/hook reactivity — which closes most of
  the velocity gap (premise 2) and the AI-tooling gap (premise 3; assistants
  handle RSX well because it mirrors JSX).
- **Dioxus is multi-target from one codebase**, not single-target: Web (WASM),
  Desktop (`wry`/`tao` webview, Tauri-like), and Mobile (iOS/Android). The "web"
  build preserves the standard browser experience (premise 4) **and** we gain
  desktop + mobile for free from the same source.
- **Type-sharing.** A Dioxus crate lives inside the Cargo workspace and imports
  `seasoned-hand-core` DTOs directly, eliminating the duplicated-type tax (and the
  `ts-rs` codegen) that ADR-002 listed under "Negative."

Three of ADR-002's four rejection grounds no longer hold; the fourth (browser
experience) is satisfied by the Dioxus web target. The remaining honest cost is
the React **component-library** ecosystem, addressed under Consequences.

## Decision

**Replace the Next.js + React + TypeScript frontend with a Dioxus (Rust)
frontend**, delivered as a new workspace crate (`crates/seasoned-hand-ui`).

- **Targets:** Web (WASM, served as today behind the server on `:3001`),
  Desktop (`dioxus-desktop`), Mobile (`dioxus-mobile`, staged last).
- **Backend contract unchanged.** The UI talks to the existing Axum `/v1` REST
  routes (`lib/api.ts` equivalents) + the WebSocket event stream (`lib/ws.ts`
  equivalent). The API boundary **is** the contract and does not change — this is
  a frontend-layer swap only; the Rust control plane is untouched.
- **Styling:** Tailwind retained (Dioxus integrates Tailwind via the standard
  CLI/PostCSS pipeline).
- This pivots **BASELINE §7 hard-decision #5** from *"Rust backend + TypeScript
  frontend (not unified language)"* to **"unified Rust, full stack."** BASELINE §4
  and §7 are amended in the same change; ARCHITECTURE.md bumps v1.4 → v1.5.

### The hard part — JS-only components (called out explicitly)

Three components in the current `AgentComputer` panel have **no native Rust
equivalent** and are deeply JS:

| Component | Role | Native Rust equivalent |
|---|---|---|
| `monaco-editor` | code editor (`editor-tab.tsx`) | none |
| `@xterm/xterm` | terminal (`terminal-tab.tsx`) | none |
| noVNC | browser-takeover VNC (`browser-tab.tsx`) | none |

Strategy:
- **Web & Desktop targets** run inside a browser/webview context, so these three
  are kept and embedded through Dioxus **JS interop** (`document::eval` / `web-sys`)
  inside dedicated wrapper components. They remain JS dependencies; we do not
  reimplement them.
- **Mobile target**: the `AgentComputer` panel degrades to a **read-only
  status/log view**. A full operator-grade terminal/editor/VNC on a phone is not a
  real use case; this matches the original "read-only mobile companion" intent.

**This is the central risk.** The migration must prove the three interop wrappers
work in `dioxus-web` **first** (see Migration plan step 1) before committing to the
full port.

## Migration plan (staged — Phase 6 stories)

1. **De-risking spike.** Stand up a `dioxus-web` skeleton hitting `/v1` (task list +
   briefing-approve) **and** prototype the Monaco / xterm / noVNC JS-interop
   wrappers. **Gate:** if the interop proves unworkable, abort the full
   replacement and fall back to the additive-companion option (Next.js retained,
   Dioxus companion for status/approvals only). This step exists to make the bet
   reversible cheaply.
2. **Port the pure-UI surface** (`project-list`, `task-list`, `chat`,
   `three-panel-layout`, briefing/approval) to RSX, sharing `seasoned-hand-core`
   DTOs.
3. **Wire the JS-interop wrappers** for the `AgentComputer` tabs (web/desktop).
4. **Add targets:** desktop (`dioxus-desktop`), then mobile (read-only
   `AgentComputer`).
5. **Cutover:** replace `frontend/` with the Dioxus crate; update
   `docker-compose.yml`, `justfile`, and `docs/`; delete the Next.js app.
6. Each step is a Phase 6 story with the usual `just verify` gates (Rust gates now
   cover the UI; the pnpm/Next pipeline is retired, a WASM build target is added).

## Consequences

**Positive:**
- One language and one toolchain across the entire stack — removes the
  two-skill-set / two-CI-pipeline tax ADR-002 listed under "Negative."
- Compile-checked types end-to-end; no TS↔Rust drift, no `ts-rs` codegen step.
- Web + desktop + mobile from a single codebase (the multi-platform goal), instead
  of a web app plus a separately-maintained companion.
- Desktop footprint is webview-based (Tauri-like), far smaller than an Electron
  alternative.
- Self-hosting simplifies: the server can embed/serve the WASM bundle; one fewer
  runtime (no Node/Next build in production).

**Negative:**
- **Loses the React component ecosystem** (shadcn/ui and mature libraries).
  Dioxus's component ecosystem is younger; some widgets must be hand-built.
- Monaco / xterm / noVNC remain **JS dependencies** reached via interop on
  web/desktop — so we do not fully escape JS, and the interop wrappers add
  complexity and a maintenance surface.
- Mobile loses the full operator console (accepted: read-only there).
- **Real rewrite cost** — the entire existing `frontend/` is replaced; the
  Next.js work is sunk.
- `dioxus-mobile` is the least mature target (risk; staged last, behind the
  desktop+web wins).

**Neutral:**
- The `/v1` REST + WebSocket boundary is unchanged; the Rust control plane,
  Bifrost, and the sandbox are untouched.
- Next.js SSR/SEO is dropped — irrelevant for an authenticated internal app;
  client-side WASM rendering is sufficient.

## Alternatives considered

### Alternative A — Keep Next.js, add a separate Dioxus companion
The original recommendation: retain the rich Next.js operator console, add a small
Dioxus companion (read-only status + approvals) for mobile/desktop. Lower risk,
additive, matches the "Beyond v1" roadmap item. **Rejected** per explicit project
decision in favour of full unification; its downside is two frontends to maintain
indefinitely. Retained as the **fallback** if the step-1 spike fails.

### Alternative B — Tauri (Rust shell wrapping the existing web UI)
Gets a desktop app cheaply by packaging the current Next.js build in a Tauri
webview. **Rejected:** still maintains the TypeScript frontend (no language
unification, no type-sharing) and does not give a clean mobile target.

### Alternative C — Flutter / React Native
Mature cross-platform UI. **Rejected:** adds a third language (Dart) or keeps JS,
and cannot share the Rust `core` DTOs — strictly worse than the status quo on the
unification goal that motivates this ADR.

### Alternative D — Stay Next.js, ship web-only + PWA for "mobile"
Lowest effort; a PWA approximates mobile. **Rejected:** no native desktop/mobile,
does not meet the multi-platform goal.

## References

- [ADR-002](ADR-002-rust-typescript-hybrid.md) — amended on its frontend clause;
  its Rust-backend clause and its "Alternative C: All-Rust" reasoning are the
  direct antecedents of this ADR.
- Dioxus: https://dioxuslabs.com
- BASELINE §4 (architecture table) + §7 hard-decision #5 — amended in this change.
- ARCHITECTURE.md §1.1 (component layers) — Frontend box updated; spec → v1.5.
