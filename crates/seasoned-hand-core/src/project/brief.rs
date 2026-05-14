//! `Brief` — the structured authored work spec the Initializer produces
//! before the Worker loop starts. Persisted on `tasks.brief` (V006) and
//! re-emitted as the `Misc{kind:"briefing"}` event the user confirms /
//! edits / cancels via the WS `user_response` cmd.
//!
//! Story 2.7 lands the type surface + JSON-schema-style validation +
//! the LLM-output parser. The actual confirm gate (and the Initializer
//! that authors a Brief) is story 2.8 / `agent::init::briefing`.
//!
//! refs: /specs/phase-2/architecture.md §2.2, §8 (over-large brief failure mode)
//! refs: /specs/phase-2/stories/story-2.7.md

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Caps from architecture §8 "Brief over-large" failure mode. Surfaced
/// as typed [`BriefError`] variants so callers can render
/// `briefing_invalid{reason}` Misc events without string sniffing.
pub const MAX_PHASES: usize = 20;
pub const MAX_SUCCESS_CRITERIA: usize = 50;
pub const MAX_DELIVERABLES: usize = 20;
pub const MAX_GOAL_LEN: usize = 4000;
pub const MAX_SUCCESS_CRITERION_LEN: usize = 200;
pub const MAX_PHASE_TITLE_LEN: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Brief {
    pub goal: String,
    pub phases: Vec<BriefPhase>,
    #[serde(default)]
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub expected_deliverables: Vec<DeliverableSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefPhase {
    pub id: u32,
    pub title: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliverableSpec {
    pub filename: String,
    pub format: DeliverableFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliverableFormat {
    Markdown,
    Json,
    Csv,
    Docx,
    Pdf,
    Html,
    Pptx,
    Xlsx,
    Code,
    Url,
}

impl DeliverableFormat {
    /// Infer renderer from the deliverable's target filename. `.txt`
    /// aliases to Markdown so plaintext deliverables ride the
    /// markdown-renderer path. Unknown extensions return `None` — the
    /// renderer dispatcher can either reject or fall back to markdown.
    pub fn from_filename(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        let ext = std::path::Path::new(&lower)
            .extension()
            .and_then(|e| e.to_str())?;
        Some(match ext {
            "md" | "txt" => Self::Markdown,
            "json" => Self::Json,
            "csv" => Self::Csv,
            "docx" => Self::Docx,
            "pdf" => Self::Pdf,
            "html" | "htm" => Self::Html,
            "pptx" => Self::Pptx,
            "xlsx" => Self::Xlsx,
            _ => return None,
        })
    }
}

#[derive(Debug, Error, Clone)]
pub enum BriefError {
    #[error("brief parse failed: {reason}")]
    ParseFailed { reason: String },
    #[error("brief invalid: {0}")]
    Invalid(&'static str),
    #[error("too many phases: {0} (max {MAX_PHASES})")]
    TooManyPhases(usize),
    #[error("too many success criteria: {0} (max {MAX_SUCCESS_CRITERIA})")]
    TooManySuccessCriteria(usize),
    #[error("too many deliverables: {0} (max {MAX_DELIVERABLES})")]
    TooManyDeliverables(usize),
    #[error("brief edited 5 times; please cancel and restart")]
    TooManyEdits,
}

impl Brief {
    pub fn validate(&self) -> Result<(), BriefError> {
        if self.goal.trim().is_empty() {
            return Err(BriefError::Invalid("goal_empty"));
        }
        if self.goal.len() > MAX_GOAL_LEN {
            return Err(BriefError::Invalid("goal_too_long"));
        }
        if self.phases.is_empty() {
            return Err(BriefError::Invalid("phases_empty"));
        }
        if self.phases.len() > MAX_PHASES {
            return Err(BriefError::TooManyPhases(self.phases.len()));
        }
        for p in &self.phases {
            if p.id == 0 {
                return Err(BriefError::Invalid("phase_id_zero"));
            }
            if p.title.trim().is_empty() {
                return Err(BriefError::Invalid("phase_title_empty"));
            }
            if p.title.len() > MAX_PHASE_TITLE_LEN {
                return Err(BriefError::Invalid("phase_title_too_long"));
            }
        }
        if self.success_criteria.len() > MAX_SUCCESS_CRITERIA {
            return Err(BriefError::TooManySuccessCriteria(
                self.success_criteria.len(),
            ));
        }
        for sc in &self.success_criteria {
            if sc.len() > MAX_SUCCESS_CRITERION_LEN {
                return Err(BriefError::Invalid("success_criterion_too_long"));
            }
        }
        if self.expected_deliverables.len() > MAX_DELIVERABLES {
            return Err(BriefError::TooManyDeliverables(
                self.expected_deliverables.len(),
            ));
        }
        Ok(())
    }

    /// Parse the planner-slot LLM's response. Accepts raw JSON or a
    /// ` ```json ... ``` ` fenced block (matches the Phase 1 planner
    /// parser's tolerance for verbose models). Missing
    /// `success_criteria` / `expected_deliverables` default to empty —
    /// the planner prompt is upgraded over time and older outputs stay
    /// parseable.
    pub fn from_planner_output(raw: &str) -> Result<Self, BriefError> {
        let trimmed = raw.trim();
        let json_text = strip_json_fence(trimmed).unwrap_or(trimmed);
        serde_json::from_str::<Self>(json_text).map_err(|e| BriefError::ParseFailed {
            reason: e.to_string(),
        })
    }

    /// Serialize for storage in `tasks.brief` (V006 JSON TEXT column).
    /// Infallible — every field uses derive-Serialize on owned types.
    pub fn serialize(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

fn strip_json_fence(raw: &str) -> Option<&str> {
    let after_open = raw
        .strip_prefix("```json")
        .or_else(|| raw.strip_prefix("```"))?;
    let body = after_open.trim_start_matches(['\n', '\r']);
    let end = body.find("```")?;
    Some(body[..end].trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_brief() -> Brief {
        Brief {
            goal: "Summarise the report".into(),
            phases: vec![
                BriefPhase {
                    id: 1,
                    title: "Plan".into(),
                    capabilities: vec!["read".into()],
                },
                BriefPhase {
                    id: 2,
                    title: "Write".into(),
                    capabilities: vec![],
                },
            ],
            success_criteria: vec!["Deliverable produced".into()],
            expected_deliverables: vec![DeliverableSpec {
                filename: "summary.md".into(),
                format: DeliverableFormat::Markdown,
                description: None,
            }],
        }
    }

    #[test]
    fn brief_serialize_round_trips() {
        let b = sample_brief();
        let value = b.serialize();
        let back: Brief = serde_json::from_value(value).expect("round-trip");
        assert_eq!(b, back);
    }

    #[test]
    fn brief_validate_accepts_well_formed_brief() {
        let b = sample_brief();
        b.validate().expect("valid");
    }

    #[test]
    fn brief_validate_rejects_empty_goal() {
        let mut b = sample_brief();
        b.goal = "   ".into();
        let err = b.validate().expect_err("empty goal");
        assert!(matches!(err, BriefError::Invalid("goal_empty")));
    }

    #[test]
    fn brief_validate_caps_phase_count() {
        let mut b = sample_brief();
        b.phases = (1..=21)
            .map(|i| BriefPhase {
                id: i,
                title: format!("p{i}"),
                capabilities: vec![],
            })
            .collect();
        let err = b.validate().expect_err("too many");
        assert!(matches!(err, BriefError::TooManyPhases(21)));
    }

    #[test]
    fn brief_validate_caps_success_criteria_count() {
        let mut b = sample_brief();
        b.success_criteria = (0..51).map(|i| format!("sc {i}")).collect();
        let err = b.validate().expect_err("too many sc");
        assert!(matches!(err, BriefError::TooManySuccessCriteria(51)));
    }

    #[test]
    fn brief_validate_caps_deliverable_count() {
        let mut b = sample_brief();
        b.expected_deliverables = (0..21)
            .map(|i| DeliverableSpec {
                filename: format!("d{i}.md"),
                format: DeliverableFormat::Markdown,
                description: None,
            })
            .collect();
        let err = b.validate().expect_err("too many deliverables");
        assert!(matches!(err, BriefError::TooManyDeliverables(21)));
    }

    #[test]
    fn deliverable_format_from_filename_dispatches_by_extension() {
        assert_eq!(
            DeliverableFormat::from_filename("report.md"),
            Some(DeliverableFormat::Markdown),
        );
        assert_eq!(
            DeliverableFormat::from_filename("notes.txt"),
            Some(DeliverableFormat::Markdown),
        );
        assert_eq!(
            DeliverableFormat::from_filename("Q4-summary.docx"),
            Some(DeliverableFormat::Docx),
        );
        assert_eq!(
            DeliverableFormat::from_filename("DATA.XLSX"),
            Some(DeliverableFormat::Xlsx),
        );
        assert_eq!(DeliverableFormat::from_filename("noext"), None);
        assert_eq!(DeliverableFormat::from_filename("weird.zzz"), None);
    }

    #[test]
    fn from_planner_output_parses_raw_json() {
        let raw = json!({
            "goal": "Build",
            "phases": [{"id": 1, "title": "Plan", "capabilities": []}],
        })
        .to_string();
        let b = Brief::from_planner_output(&raw).expect("parses raw");
        assert_eq!(b.goal, "Build");
        assert_eq!(b.phases.len(), 1);
        // Defaults applied when absent.
        assert!(b.success_criteria.is_empty());
        assert!(b.expected_deliverables.is_empty());
    }

    #[test]
    fn from_planner_output_parses_fenced_json() {
        let raw = "```json\n{\"goal\":\"Build\",\"phases\":[{\"id\":1,\"title\":\"Plan\"}]}\n```";
        let b = Brief::from_planner_output(raw).expect("parses fenced");
        assert_eq!(b.goal, "Build");
        assert_eq!(b.phases[0].id, 1);
    }

    #[test]
    fn from_planner_output_returns_parse_failed_on_garbage() {
        let err = Brief::from_planner_output("not json").expect_err("garbage");
        assert!(matches!(err, BriefError::ParseFailed { .. }));
    }
}
