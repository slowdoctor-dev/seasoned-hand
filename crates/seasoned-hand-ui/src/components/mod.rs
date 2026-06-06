//! Component tree. `App` provides shared state (the agent socket + the current
//! Project/Task/Session selection) via context, mirroring the responsibilities
//! the old `HomeShell` held in React.

use dioxus::prelude::*;

mod agent_computer;
mod briefing_card;
mod chat;
mod project_list;
mod task_list;
mod three_panel;

use crate::ws::{use_agent_socket, AgentSocket};
use three_panel::ThreePanel;

/// The active selection shared across the three panels. All fields are `Copy`
/// signal handles, so `Selection` is cheaply shared through context (replaces
/// the `useState` trio that `HomeShell` threaded as props).
#[derive(Clone, Copy)]
pub struct Selection {
    pub active_project: Signal<Option<String>>,
    pub active_task: Signal<Option<String>>,
    pub session_id: Signal<Option<String>>,
}

#[component]
pub fn App() -> Element {
    // Selection must exist before the socket so the coroutine can write the
    // session_id captured from a task_create ack.
    let selection = Selection {
        active_project: use_signal(|| None),
        active_task: use_signal(|| None),
        session_id: use_signal(|| None),
    };
    use_context_provider(|| selection);

    let socket = use_agent_socket(selection.session_id);
    use_context_provider(|| socket.clone());

    rsx! { ThreePanel {} }
}

/// Convenience accessors used by the panels.
pub fn selection() -> Selection {
    use_context::<Selection>()
}

pub fn socket() -> AgentSocket {
    use_context::<AgentSocket>()
}
