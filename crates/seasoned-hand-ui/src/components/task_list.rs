//! Task list (replaces `task-list.tsx`). Loads `/v1/projects/:id/tasks` for the
//! active project and drives the `active_task` selection.

use super::selection;
use crate::api;
use dioxus::prelude::*;
use seasoned_hand_dto::TaskStatus;

fn status_label(s: &TaskStatus) -> &'static str {
    match s {
        TaskStatus::Drafted => "drafted",
        TaskStatus::Briefed => "briefed",
        TaskStatus::Confirmed => "confirmed",
        TaskStatus::Running => "running",
        TaskStatus::Paused => "paused",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Cancelled => "cancelled",
    }
}

#[component]
pub fn TaskList() -> Element {
    let sel = selection();
    let active_project = sel.active_project;
    let mut active_task = sel.active_task;

    let tasks = use_resource(move || async move {
        match active_project() {
            Some(pid) => api::list_tasks(&pid, 50).await,
            None => Ok(Vec::new()),
        }
    });

    rsx! {
        div { class: "h-full overflow-y-auto p-2",
            div { class: "px-1 pb-1 text-xs uppercase tracking-wide text-neutral-500", "Tasks" }
            match &*tasks.read_unchecked() {
                Some(Ok(list)) if !list.is_empty() => {
                    let items = list.iter().map(|t| {
                        let tid = t.id.clone();
                        let title = t.title.clone();
                        let status = status_label(&t.status);
                        let active = active_task() == Some(tid.clone());
                        let cls = if active {
                            "w-full rounded px-2 py-1 text-left bg-neutral-800"
                        } else {
                            "w-full rounded px-2 py-1 text-left hover:bg-neutral-900"
                        };
                        rsx! {
                            li { key: "{tid}",
                                button {
                                    class: "{cls}",
                                    onclick: {
                                        let tid = tid.clone();
                                        move |_| active_task.set(Some(tid.clone()))
                                    },
                                    div { class: "truncate", "{title}" }
                                    div { class: "text-xs text-neutral-500", "{status}" }
                                }
                            }
                        }
                    }).collect::<Vec<_>>();
                    rsx! { ul { class: "space-y-0.5", {items.into_iter()} } }
                }
                Some(Ok(_)) => rsx! { div { class: "px-2 py-1 text-neutral-600", "No tasks" } },
                Some(Err(e)) => rsx! { div { class: "px-2 py-1 text-red-400", "Failed: {e}" } },
                None => rsx! { div { class: "px-2 py-1 text-neutral-500", "Loading…" } },
            }
        }
    }
}
