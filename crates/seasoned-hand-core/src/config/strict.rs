//! Strict-parse helpers for SH_* environment variables (Phase 5 story
//! 5.22; lifted from the server crate's main.rs introduced in Phase 4
//! story 4.14).
//!
//! "Strict" means: invalid values return `Err(String)` with a
//! structured human-readable message instead of silently coercing to
//! a default. The convention across the codebase is that startup paths
//! propagate that error to `Box<dyn std::error::Error>` so misconfigured
//! deployments fail fast at boot rather than silently disabling
//! security boundaries.
//!
//! Why this lives in core: server, CLI, and any worker spawn that
//! reads SH_* vars share one parser. Independent reimplementations
//! drift; this module is the single source of truth.
//!
//! refs: /specs/phase-5/stories/story-5.22.md
//! refs: /specs/phase-5/requirements.md F-5.18, NFR-5.7
//! closes: DEBT #91

/// Strict bool parser. Accepts `1`, `0`, `true`, `false` (case-
/// insensitive, surrounding whitespace trimmed). Any other value
/// returns `Err`. Use for `SH_*_ENABLED`-style flags.
pub fn parse_bool_strict(name: &str, raw: &str) -> Result<bool, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => Err(format!(
            "{name} invalid boolean value '{raw}' (expected one of: 1, 0, true, false)"
        )),
    }
}

pub fn parse_u64_strict(name: &str, raw: &str) -> Result<u64, String> {
    raw.trim()
        .parse::<u64>()
        .map_err(|_| format!("{name} invalid unsigned integer value '{raw}'"))
}

pub fn parse_u32_strict(name: &str, raw: &str) -> Result<u32, String> {
    raw.trim()
        .parse::<u32>()
        .map_err(|_| format!("{name} invalid unsigned integer value '{raw}'"))
}

pub fn parse_f32_strict(name: &str, raw: &str) -> Result<f32, String> {
    raw.trim()
        .parse::<f32>()
        .map_err(|_| format!("{name} invalid float value '{raw}'"))
}

/// Look up `name` via the caller-supplied lookup closure and run the
/// strict bool parser. Returns `default` when the var is unset. The
/// lookup closure indirection lets tests inject synthetic env maps
/// without touching the process-global `std::env`.
pub fn env_bool_or_default<F>(lookup: &F, name: &str, default: bool) -> Result<bool, String>
where
    F: Fn(&str) -> Option<String>,
{
    match lookup(name) {
        Some(raw) => parse_bool_strict(name, &raw),
        None => Ok(default),
    }
}

pub fn env_u64_or_default<F>(lookup: &F, name: &str, default: u64) -> Result<u64, String>
where
    F: Fn(&str) -> Option<String>,
{
    match lookup(name) {
        Some(raw) => parse_u64_strict(name, &raw),
        None => Ok(default),
    }
}

pub fn env_u32_or_default<F>(lookup: &F, name: &str, default: u32) -> Result<u32, String>
where
    F: Fn(&str) -> Option<String>,
{
    match lookup(name) {
        Some(raw) => parse_u32_strict(name, &raw),
        None => Ok(default),
    }
}

pub fn env_f32_or_default<F>(lookup: &F, name: &str, default: f32) -> Result<f32, String>
where
    F: Fn(&str) -> Option<String>,
{
    match lookup(name) {
        Some(raw) => parse_f32_strict(name, &raw),
        None => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(map: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + use<'a> {
        move |k| {
            map.iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn parse_bool_accepts_canonical_forms() {
        for v in ["1", "true", "TRUE", "True", " true ", "tRuE"] {
            assert!(parse_bool_strict("X", v).is_ok());
        }
        for v in ["0", "false", "FALSE", " 0 "] {
            assert!(parse_bool_strict("X", v).is_ok());
        }
    }

    #[test]
    fn parse_bool_rejects_other_values_with_named_error() {
        let err = parse_bool_strict("SH_X", "yes").expect_err("must reject 'yes'");
        assert!(err.contains("SH_X"));
        assert!(err.contains("'yes'"));
    }

    #[test]
    fn parse_u64_rejects_non_numeric() {
        let err = parse_u64_strict("SH_INTERVAL", "five").expect_err("must reject text");
        assert!(err.contains("SH_INTERVAL"));
    }

    #[test]
    fn env_bool_or_default_uses_default_when_unset() {
        let l = lookup(&[]);
        assert_eq!(env_bool_or_default(&l, "SH_X", true), Ok(true));
        assert_eq!(env_bool_or_default(&l, "SH_X", false), Ok(false));
    }

    #[test]
    fn env_bool_or_default_strict_parses_when_set() {
        let l = lookup(&[("SH_X", "false")]);
        assert_eq!(env_bool_or_default(&l, "SH_X", true), Ok(false));
    }

    #[test]
    fn env_u64_or_default_strict_rejects_invalid_value() {
        let l = lookup(&[("SH_INTERVAL", "not-a-number")]);
        let err = env_u64_or_default(&l, "SH_INTERVAL", 60).expect_err("must reject");
        assert!(err.contains("SH_INTERVAL"));
    }

    #[test]
    fn env_u32_and_f32_share_the_same_strict_semantics() {
        let l_u = lookup(&[("SH_N", "9999")]);
        assert_eq!(env_u32_or_default(&l_u, "SH_N", 0), Ok(9999u32));
        let l_f = lookup(&[("SH_PCT", "0.42")]);
        assert!((env_f32_or_default(&l_f, "SH_PCT", 0.0).unwrap() - 0.42).abs() < 1e-6);
    }
}
