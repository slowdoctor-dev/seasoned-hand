//! Project list (replaces `project-list.tsx`). Loads `/v1/projects` and drives
//! the shared `active_project` selection.

use super::selection;
use crate::api;
use dioxus::prelude::*;

#[component]
pub fn ProjectList() -> Element {
    let sel = selection();
    let mut active_project = sel.active_project;
    let mut active_task = sel.active_task;

    let projects = use_resource(|| async move { api::list_projects(50).await });

    rsx! {
        div { class: "border-b border-neutral-800 p-2",
            div { class: "px-1 pb-1 text-xs uppercase tracking-wide text-neutral-500",
                "Projects"
            }
            match &*projects.read_unchecked() {
                Some(Ok(list)) => {
                    let items = list.iter().map(|p| {
                        let pid = p.id.clone();
                        let title = p.title.clone();
                        let active = active_project() == Some(pid.clone());
                        let cls = if active {
                            "w-full rounded px-2 py-1 text-left bg-neutral-800"
                        } else {
                            "w-full rounded px-2 py-1 text-left hover:bg-neutral-900"
                        };
                        rsx! {
                            li { key: "{pid}",
                                button {
                                    class: "{cls}",
                                    onclick: {
                                        let pid = pid.clone();
                                        move |_| {
                                            active_project.set(Some(pid.clone()));
                                            active_task.set(None);
                                        }
                                    },
                                    "{title}"
                                }
                            }
                        }
                    }).collect::<Vec<_>>();
                    rsx! { ul { class: "space-y-0.5", {items.into_iter()} } }
                }
                Some(Err(e)) => rsx! { div { class: "px-2 py-1 text-red-400", "Failed: {e}" } },
                None => rsx! { div { class: "px-2 py-1 text-neutral-500", "Loading…" } },
            }
        }
    }
}
