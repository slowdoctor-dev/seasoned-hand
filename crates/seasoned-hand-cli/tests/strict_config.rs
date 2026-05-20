//! Story 5.22 CLI strict-config regression tests.
//!
//! The CLI's typed env vars (e.g. retry counts, polling intervals when
//! we add them) MUST use the same `seasoned_hand_core::config::strict`
//! helpers as the server so deployments can't accidentally land a
//! permissive parse in one binary but not the other.
//!
//! refs: /specs/phase-5/stories/story-5.22.md

use seasoned_hand_core::config::strict::{env_bool_or_default, env_u32_or_default};

fn lookup<'a>(entries: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + use<'a> {
    move |k| {
        entries
            .iter()
            .find(|(name, _)| *name == k)
            .map(|(_, v)| (*v).to_string())
    }
}

#[test]
fn cli_strict_parse_rejects_invalid_typed_var() {
    let env = lookup(&[("SH_CLI_RETRIES", "many")]);
    let err = env_u32_or_default(&env, "SH_CLI_RETRIES", 3).expect_err("must reject");
    assert!(err.contains("SH_CLI_RETRIES"));
}

#[test]
fn cli_strict_parse_accepts_valid_typed_var() {
    let env = lookup(&[("SH_CLI_RETRIES", "5")]);
    assert_eq!(env_u32_or_default(&env, "SH_CLI_RETRIES", 3), Ok(5));
}

#[test]
fn cli_strict_bool_rejects_yes_no() {
    // Same DEBT #91 motivation as the server tests: any "permissive"
    // value (yes/on/y) fails fast at boot.
    let env = lookup(&[("SH_CLI_DEBUG", "yes")]);
    assert!(env_bool_or_default(&env, "SH_CLI_DEBUG", false).is_err());
    let env = lookup(&[("SH_CLI_DEBUG", "no")]);
    assert!(env_bool_or_default(&env, "SH_CLI_DEBUG", false).is_err());
}
