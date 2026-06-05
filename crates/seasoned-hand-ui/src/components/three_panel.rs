//! 3-panel layout shell (replaces `three-panel-layout.tsx` + `home-shell.tsx`).

use super::agent_computer::AgentComputer;
use super::chat::Chat;
use super::project_list::ProjectList;
use super::task_list::TaskList;
use dioxus::prelude::*;

#[component]
pub fn ThreePanel() -> Element {
    rsx! {
        div { class: "flex h-screen w-screen overflow-hidden bg-neutral-950 text-neutral-100 text-sm",
            aside { class: "flex w-72 shrink-0 flex-col border-r border-neutral-800",
                ProjectList {}
                div { class: "min-h-0 flex-1 overflow-hidden", TaskList {} }
            }
            main { class: "flex min-w-0 flex-1 flex-col border-r border-neutral-800",
                Chat {}
            }
            section { class: "flex w-2/5 min-w-0 flex-col", AgentComputer {} }
        }
    }
}
