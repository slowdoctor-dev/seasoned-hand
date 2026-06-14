//! Invitation-token login form (issue #26). Shown when no stored session exists
//! and dev-login is unavailable (non-loopback / production server). Exchanges a
//! single-use invitation token for a verified session via `/v1/auth/login`.

use dioxus::prelude::*;

use crate::auth::{self, AuthState};

#[component]
pub fn Login(error: Option<String>) -> Element {
    let mut invitation = use_signal(String::new);
    let mut err = use_signal(|| error);
    let mut submitting = use_signal(|| false);

    rsx! {
        div { class: "flex h-screen w-screen items-center justify-center bg-neutral-950 text-neutral-100 text-sm",
            div { class: "flex w-80 flex-col gap-3 rounded-lg border border-neutral-800 bg-neutral-900 p-6",
                h1 { class: "text-lg font-semibold", "Sign in" }
                p { class: "text-neutral-400", "Enter your invitation token to continue." }
                input {
                    class: "rounded bg-neutral-950 px-2 py-1.5 outline-none border border-neutral-800 focus:border-blue-600",
                    r#type: "password",
                    placeholder: "invitation token",
                    value: "{invitation}",
                    autofocus: true,
                    oninput: move |e| invitation.set(e.value()),
                }
                button {
                    class: "rounded bg-blue-600 px-3 py-1.5 font-medium hover:bg-blue-500 disabled:opacity-50",
                    disabled: submitting(),
                    onclick: move |_| {
                        if submitting() {
                            return;
                        }
                        let token = invitation().trim().to_string();
                        if token.is_empty() {
                            err.set(Some("Enter an invitation token.".to_string()));
                            return;
                        }
                        submitting.set(true);
                        err.set(None);
                        spawn(async move {
                            match auth::login(&token).await {
                                Ok(session) => {
                                    auth::store_session(&session);
                                    *auth::AUTH.write() = AuthState::Authed;
                                }
                                Err(_) => {
                                    err.set(Some(
                                        "Invalid or expired invitation token.".to_string(),
                                    ));
                                    submitting.set(false);
                                }
                            }
                        });
                    },
                    if submitting() { "Signing in…" } else { "Sign in" }
                }
                if let Some(msg) = err() {
                    p { class: "text-red-400", "{msg}" }
                }
            }
        }
    }
}
