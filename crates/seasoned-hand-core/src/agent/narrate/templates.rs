//! Templated narration strings. Per-tool match with a generic fallback.
//!
//! Narration style (architecture §2.8): imperative, no period, no
//! technical jargon ("invoke", "execute" except in the generic
//! fallback where the tool name is opaque), 8-15 words.

use serde_json::Value;

pub fn template_for(tool: &str, args: &Value) -> String {
    let pick_str = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    match tool {
        "plan_advance" => "Advancing the plan".into(),
        "plan_update" => "Updating the plan".into(),
        "plan_create" => "Drafting the plan".into(),
        "idle" => "Wrapping up".into(),
        "feature_mark_done" => match pick_str("feature_id") {
            Some(id) => format!("Marking feature {id} done"),
            None => "Marking a feature done".into(),
        },
        "progress_update" => "Logging progress".into(),
        "checkpoint_label" => "Labeling next checkpoint".into(),
        "file_read" => match pick_str("path") {
            Some(path) => format!("Reading {path}"),
            None => "Reading a file".into(),
        },
        "file_find_by_name" => "Searching workspace for a file".into(),
        "file_find_in_content" => "Searching workspace content".into(),
        "glossary_lookup" => "Looking up the glossary".into(),
        "playbook_search" => "Searching playbooks".into(),
        "sop_read" => "Reading an SOP".into(),
        _ => format!("Invoking {tool}"),
    }
}
