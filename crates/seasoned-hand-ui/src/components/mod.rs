//! Component tree. `App` provides shared state (the agent socket + the current
//! Project/Task/Session selection) via context, mirroring the responsibilities
//! the old `HomeShell` held in React.

use dioxus::prelude::*;

mod agent_computer;
mod briefing_card;
mod chat;
mod login;
mod project_list;
mod task_list;
mod three_panel;

use crate::auth::{self, AuthState};
use crate::ws::{use_agent_socket, AgentSocket};
use login::Login;
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
    // Issue #26: bootstrap a verified session once on mount — reuse a stored
    // session, else try zero-friction dev-login, else fall back to the
    // invitation-token form.
    use_future(|| async move {
        if auth::load_session().is_some() {
            *auth::AUTH.write() = AuthState::Authed;
            return;
        }
        match auth::dev_login().await {
            Ok(session) => {
                auth::store_session(&session);
                *auth::AUTH.write() = AuthState::Authed;
            }
            Err(_) => *auth::AUTH.write() = AuthState::NeedLogin(None),
        }
    });

    match auth::AUTH() {
        AuthState::Loading => rsx! {
            div { class: "flex h-screen w-screen items-center justify-center bg-neutral-950 text-neutral-400 text-sm",
                "Authenticating…"
            }
        },
        AuthState::NeedLogin(error) => rsx! { Login { error } },
        // The socket lives INSIDE the authed subtree (keyed to the session): it is
        // created only once a token exists — so no pre-auth backoff wait — and is
        // torn down when auth is cleared (401 / sign-out), closing the old
        // WebSocket and dropping its subscription state so a re-login gets a fresh
        // socket under the new identity rather than reusing the old upgrade context.
        AuthState::Authed => rsx! { AuthedApp {} },
    }
}

/// The signed-in app: owns the per-session selection + agent socket and renders
/// the three-panel console. Unmounting it (on sign-out / 401) drops the socket
/// coroutine, which closes the underlying WebSocket.
#[component]
fn AuthedApp() -> Element {
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
