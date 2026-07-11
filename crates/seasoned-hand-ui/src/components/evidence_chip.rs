//! Evidence chip (issue #3, ports the removed React `evidence-chip.tsx`).
//! A verifier verdict's `evidence_event_ids` render as chips; clicking one
//! expands the full event JSON, resolved O(1) from the per-session event index
//! built in the AgentComputer panel (parity with `HomeShell`'s `eventIndex`).

use dioxus::prelude::*;
use seasoned_hand_dto::ServerEvent;

#[component]
pub fn EvidenceChip(event_id: i64, event: Option<ServerEvent>) -> Element {
    let mut open = use_signal(|| false);

    let Some(ev) = event else {
        return rsx! {
            span {
                class: "cursor-default rounded bg-neutral-800 px-2 py-0.5 text-[11px] text-neutral-500",
                title: "Event is older than the currently loaded window",
                "#{event_id} (older than loaded window)"
            }
        };
    };

    let pretty = serde_json::to_string_pretty(&ev.payload).unwrap_or_default();
    rsx! {
        span { class: "inline-flex flex-col items-start",
            button {
                class: "rounded bg-blue-900 px-2 py-0.5 text-[11px] text-blue-100 hover:bg-blue-800",
                onclick: move |_| {
                    let now = open();
                    open.set(!now);
                },
                "#{event_id}"
            }
            if open() {
                pre { class: "mt-1 max-h-40 max-w-md overflow-auto rounded bg-neutral-950 p-2 text-[10px] text-neutral-300",
                    "{pretty}"
                }
            }
        }
    }
}
