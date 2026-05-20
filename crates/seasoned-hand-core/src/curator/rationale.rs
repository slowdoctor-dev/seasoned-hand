//! Curator decision rationale schema versioning (Phase 5 story 5.25 /
//! closes DEBT #96).
//!
//! Phase 4 wrote `curator_decisions.rationale_json` as a flat JSON
//! object with a `policy_version` string field. Phase 5+ writes wrap
//! that inside an outer envelope so readers can dispatch on schema
//! version without parsing the inner payload first:
//!
//! ```json
//! {"schema_version": 2, "data": { ...payload... }}
//! ```
//!
//! Readers MUST tolerate both shapes. The detector [`SchemaVersion::detect`]
//! returns `V1` for the legacy flat shape (no outer `schema_version`
//! key) and `V2` for the wrapped shape. Per-version
//! [`SchemaVersion::validate`] returns the inner `data` value or an
//! error if the payload doesn't satisfy that version's required-keys
//! contract.
//!
//! refs: /specs/phase-5/architecture.md §13 (amendments)
//! refs: /specs/phase-5/stories/story-5.25.md F-5.15/F-5.16/F-5.17
//! closes: DEBT #96

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Rationale envelope versions. Bumping this enum on every shape
/// change forces every reader through the central dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchemaVersion {
    /// Phase 4 flat shape: `{ "policy_version": "...", ...fields... }`.
    /// No outer envelope. Detected by the absence of a `schema_version`
    /// integer field.
    V1,
    /// Phase 5+ wrapped shape: `{ "schema_version": 2, "data": {...} }`.
    /// All new writes use this shape.
    V2,
}

impl SchemaVersion {
    /// Inspect a rationale JSON value and return its envelope version.
    /// Falls back to `V1` when the outer shape doesn't have an integer
    /// `schema_version` key — this keeps Phase 4 rows readable without
    /// migration.
    pub fn detect(payload: &serde_json::Value) -> Self {
        match payload.get("schema_version").and_then(|v| v.as_i64()) {
            Some(2) => Self::V2,
            _ => Self::V1,
        }
    }

    pub fn as_i64(self) -> i64 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
        }
    }

    /// Validate `payload` against this version's contract and return
    /// the inner `data` value (unwrapped from the V2 envelope; the V1
    /// payload IS the data).
    pub fn validate(
        self,
        payload: &serde_json::Value,
    ) -> Result<&serde_json::Value, RationaleError> {
        match self {
            Self::V1 => {
                // V1 contract: must be a JSON object with at least one
                // recognized key. We don't enforce specific keys here
                // since Phase 4 wrote many decision-type-specific
                // shapes; the bar is just "well-formed JSON object".
                if payload.is_object() {
                    Ok(payload)
                } else {
                    Err(RationaleError::NotAnObject)
                }
            }
            Self::V2 => {
                let obj = payload.as_object().ok_or(RationaleError::NotAnObject)?;
                let version_field = obj
                    .get("schema_version")
                    .ok_or(RationaleError::MissingField("schema_version"))?;
                if version_field.as_i64() != Some(2) {
                    return Err(RationaleError::WrongVersion {
                        expected: 2,
                        observed: version_field.clone(),
                    });
                }
                obj.get("data").ok_or(RationaleError::MissingField("data"))
            }
        }
    }

    /// Build a V2-wrapped envelope around `data`. Use this at every
    /// write site so the schema_version + data shape stays consistent.
    pub fn wrap_v2(data: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 2,
            "data": data,
        })
    }
}

#[derive(Debug, Clone, Error)]
pub enum RationaleError {
    #[error("rationale payload is not a JSON object")]
    NotAnObject,
    #[error("rationale payload missing required field: {0}")]
    MissingField(&'static str),
    #[error("wrong schema_version: expected {expected}, observed {observed}")]
    WrongVersion {
        expected: i64,
        observed: serde_json::Value,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detect_v1_flat_shape() {
        // Phase 4 wrote rationale this way — flat object, no envelope.
        let payload = json!({
            "policy_version": "phase4_story_4_8",
            "pattern_key": "stripe_refund",
            "score": 0.85,
        });
        assert_eq!(SchemaVersion::detect(&payload), SchemaVersion::V1);
    }

    #[test]
    fn detect_v2_wrapped_shape() {
        let payload = json!({
            "schema_version": 2,
            "data": {"policy_version": "phase5_story_5_25", "k": "v"},
        });
        assert_eq!(SchemaVersion::detect(&payload), SchemaVersion::V2);
    }

    #[test]
    fn detect_unknown_version_falls_back_to_v1() {
        // A future Phase 6 might write schema_version=3. Older readers
        // (this binary) should still recognize the envelope shape
        // exists — but since `detect` returns V1 for the unknown case,
        // the validator on V1 will succeed if it's a well-formed
        // object, which is the right backward-compat posture: we don't
        // pretend to understand a future schema, but we don't crash on
        // it either.
        let payload = json!({
            "schema_version": 99,
            "data": {"k": "v"},
        });
        assert_eq!(SchemaVersion::detect(&payload), SchemaVersion::V1);
    }

    #[test]
    fn validate_v1_passes_well_formed_object() {
        let payload = json!({"policy_version": "x"});
        let inner = SchemaVersion::V1.validate(&payload).unwrap();
        assert_eq!(inner, &payload);
    }

    #[test]
    fn validate_v1_rejects_non_object() {
        let payload = json!("not an object");
        let err = SchemaVersion::V1
            .validate(&payload)
            .expect_err("must reject");
        assert!(matches!(err, RationaleError::NotAnObject));
    }

    #[test]
    fn validate_v2_unwraps_data() {
        let data = json!({"policy_version": "phase5", "k": "v"});
        let wrapped = SchemaVersion::wrap_v2(data.clone());
        let inner = SchemaVersion::V2.validate(&wrapped).unwrap();
        assert_eq!(inner, &data);
    }

    #[test]
    fn validate_v2_rejects_missing_data_field() {
        let payload = json!({"schema_version": 2});
        let err = SchemaVersion::V2
            .validate(&payload)
            .expect_err("missing data field must reject");
        assert!(matches!(err, RationaleError::MissingField("data")));
    }

    #[test]
    fn validate_v2_rejects_wrong_version() {
        let payload = json!({"schema_version": 1, "data": {}});
        let err = SchemaVersion::V2
            .validate(&payload)
            .expect_err("v1-versioned payload must not validate as V2");
        assert!(matches!(err, RationaleError::WrongVersion { .. }));
    }

    #[test]
    fn wrap_v2_idempotent_via_detect_validate() {
        // Round-trip: wrap → detect → validate → unwrap should yield
        // the original data.
        let data = json!({"pattern_key": "k", "score": 0.7});
        let env = SchemaVersion::wrap_v2(data.clone());
        let version = SchemaVersion::detect(&env);
        let unwrapped = version.validate(&env).unwrap();
        assert_eq!(unwrapped, &data);
        assert_eq!(version, SchemaVersion::V2);
    }
}
