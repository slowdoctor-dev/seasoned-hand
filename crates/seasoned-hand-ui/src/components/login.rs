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
        div { class: "auth-screen",
            div { class: "login-card",
                h1 { "Sign in" }
                p { "Enter your invitation token to continue." }
                input {
                    r#type: "password",
                    placeholder: "invitation token",
                    value: "{invitation}",
                    autofocus: true,
                    oninput: move |e| invitation.set(e.value()),
                }
                button {
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
                    p { class: "login-error", "{msg}" }
                }
            }
        }
    }
}
