//! Strict JSON parse of the verifier LLM's verdict envelope.
//!
//! Schema per architecture §2.4.3:
//! ```text
//! {
//!   "verdict": "pass" | "fail",
//!   "reason": "<one sentence>",
//!   "evidence_event_ids": [<u64>, ...],
//!   "suggested_plan_update": { "phases": [...] } | null
//! }
//! ```
//!
//! refs: /specs/phase-1/stories/story-1.9b.md
//! refs: /specs/phase-1/architecture.md §2.4.3, §8 ("verifier_unparseable")

use serde::Deserialize;
use serde_json::Value;

use super::VerdictKind;

/// Parsed verdict envelope. The runtime keeps `evidence_event_ids` as
/// `Vec<i64>` to match the DB column type; `suggested_plan_update` stays
/// as opaque JSON `Value` because the structure of "phases" is part of
/// the Plan module, not the verifier.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Verdict {
    pub verdict: VerdictKind,
    pub reason: String,
    #[serde(default)]
    pub evidence_event_ids: Vec<i64>,
    #[serde(default)]
    pub suggested_plan_update: Option<Value>,
}

impl Verdict {
    /// Synthesized verdict for the "verifier returned unparseable
    /// output twice in a row" fallback path (architecture §8). Stored
    /// just like a real verdict on the `verifications` row so the
    /// trigger Gate (story 1.10) sees a uniform contract.
    pub fn unparseable() -> Self {
        Self {
            verdict: VerdictKind::Fail,
            reason: "verifier_unparseable".to_string(),
            evidence_event_ids: Vec::new(),
            suggested_plan_update: None,
        }
    }
}

/// Parse a raw LLM content string as a Verdict. Returns `None` on
/// malformed JSON or schema mismatch — the caller decides whether to
/// retry or fall back to [`Verdict::unparseable`].
pub fn parse_verdict(content: &str) -> Option<Verdict> {
    // Permit models that wrap the JSON in ```json ... ``` fences or
    // prose around it. We try a direct parse first, then a best-effort
    // extraction of the first `{...}` block.
    if let Ok(v) = serde_json::from_str::<Verdict>(content) {
        return Some(v);
    }
    let first_brace = content.find('{')?;
    let last_brace = content.rfind('}')?;
    if last_brace <= first_brace {
        return None;
    }
    let candidate = &content[first_brace..=last_brace];
    serde_json::from_str::<Verdict>(candidate).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_pass_verdict_with_evidence() {
        let raw = r#"{"verdict":"pass","reason":"all green","evidence_event_ids":[1,2,3]}"#;
        let v = parse_verdict(raw).expect("parse");
        assert_eq!(v.verdict, VerdictKind::Pass);
        assert_eq!(v.reason, "all green");
        assert_eq!(v.evidence_event_ids, vec![1, 2, 3]);
        assert!(v.suggested_plan_update.is_none());
    }

    #[test]
    fn parses_fail_verdict_with_suggested_update() {
        let raw = json!({
            "verdict": "fail",
            "reason": "missing tests",
            "evidence_event_ids": [],
            "suggested_plan_update": {"phases": [{"id": 1, "title": "Add tests"}]}
        })
        .to_string();
        let v = parse_verdict(&raw).expect("parse");
        assert_eq!(v.verdict, VerdictKind::Fail);
        let suggested = v.suggested_plan_update.expect("suggested");
        assert_eq!(suggested["phases"][0]["title"], json!("Add tests"));
    }

    #[test]
    fn extracts_json_from_prose_wrapper() {
        let raw = "Here is my verdict:\n```json\n{\"verdict\":\"pass\",\"reason\":\"ok\",\"evidence_event_ids\":[]}\n```\nThanks!";
        let v = parse_verdict(raw).expect("parse despite prose wrapper");
        assert_eq!(v.verdict, VerdictKind::Pass);
    }

    #[test]
    fn returns_none_on_malformed_json() {
        assert!(parse_verdict("not json at all").is_none());
        assert!(parse_verdict("{ broken json").is_none());
        assert!(parse_verdict("").is_none());
    }

    #[test]
    fn unparseable_fallback_is_fail_with_documented_reason() {
        let v = Verdict::unparseable();
        assert_eq!(v.verdict, VerdictKind::Fail);
        assert_eq!(v.reason, "verifier_unparseable");
        assert!(v.evidence_event_ids.is_empty());
        assert!(v.suggested_plan_update.is_none());
    }
}
