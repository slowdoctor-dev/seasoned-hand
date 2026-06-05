//! Seasoned Hand — unified-Rust (Dioxus) frontend entry point (ADR-016).
//!
//! Replaces the Next.js `app/` + `components/` tree. The 3-panel operator
//! console (Projects/Tasks · Chat · AgentComputer) is rebuilt in RSX against
//! the unchanged `/v1` REST + `/ws` WebSocket boundary.
//!
//! Foundation note: some DTO/API surface (deliverables, extra commands, session
//! listing) is defined ahead of use — the follow-up Phase 6 stories that build
//! the deliverables/verifier/briefing flows consume it. `dead_code` is allowed
//! crate-wide for that reason; tighten it as each story lands.
#![allow(dead_code)]

mod api;
mod components;
mod config;
mod interop;
mod ws;

use components::App;

fn main() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Info);
    dioxus::launch(App);
}
