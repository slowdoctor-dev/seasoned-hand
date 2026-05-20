//! Story 5.22 boot-time strict-config regression tests.
//!
//! The server's `main.rs` uses the shared
//! `seasoned_hand_core::config::strict::env_*_or_default` helpers for
//! every SH_* typed env var. These tests pin the contract: invalid
//! values surface a structured error that names the offending var so
//! ops can find the misconfig quickly. Closes Phase 5 DEBT #91.
//!
//! refs: /specs/phase-5/stories/story-5.22.md

use seasoned_hand_core::config::strict::{env_bool_or_default, env_u64_or_default};

fn lookup<'a>(entries: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + use<'a> {
    move |k| {
        entries
            .iter()
            .find(|(name, _)| *name == k)
            .map(|(_, v)| (*v).to_string())
    }
}

#[test]
fn server_boot_rejects_invalid_user_cost_interval() {
    let env = lookup(&[("SH_USER_COST_INTERVAL_SEC", "not-a-number")]);
    let err = env_u64_or_default(&env, "SH_USER_COST_INTERVAL_SEC", 3600)
        .expect_err("must reject non-numeric interval");
    assert!(err.contains("SH_USER_COST_INTERVAL_SEC"));
}

#[test]
fn server_boot_accepts_valid_user_cost_interval() {
    let env = lookup(&[("SH_USER_COST_INTERVAL_SEC", "1800")]);
    assert_eq!(
        env_u64_or_default(&env, "SH_USER_COST_INTERVAL_SEC", 3600),
        Ok(1800)
    );
}

#[test]
fn server_boot_rejects_invalid_learning_enabled_value() {
    // SH_LEARNING_ENABLED previously treated "yes"/"on"/anything-not-
    // "false" as `true`, which is the exact kind of permissive parse
    // DEBT #91 closes. Strict mode rejects anything outside
    // {1,0,true,false}.
    let env = lookup(&[("SH_LEARNING_ENABLED", "yes")]);
    let err =
        env_bool_or_default(&env, "SH_LEARNING_ENABLED", true).expect_err("must reject 'yes'");
    assert!(err.contains("SH_LEARNING_ENABLED"));
}

#[test]
fn server_boot_accepts_canonical_bool_forms() {
    for raw in ["1", "0", "true", "false", "TRUE", "False"] {
        let entries: [(&str, &str); 1] = [("SH_LEARNING_ENABLED", raw)];
        let env = lookup(&entries);
        assert!(
            env_bool_or_default(&env, "SH_LEARNING_ENABLED", true).is_ok(),
            "raw '{raw}' must parse cleanly"
        );
    }
}

#[test]
fn unset_var_falls_through_to_default() {
    let env = lookup(&[]);
    assert_eq!(
        env_u64_or_default(&env, "SH_USER_COST_INTERVAL_SEC", 3600),
        Ok(3600)
    );
    assert_eq!(
        env_bool_or_default(&env, "SH_LEARNING_ENABLED", true),
        Ok(true)
    );
}
