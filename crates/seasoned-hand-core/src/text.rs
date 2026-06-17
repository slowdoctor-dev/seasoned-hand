//! Small string helpers shared across crates.
//!
//! Like [`crate::time`], this exists to collapse copies that had drifted
//! into several modules (`deliverable`, `channel`, `sandbox`) — keeping
//! one definition so the behaviour stays consistent.

/// Truncate `s` to at most `n` Unicode scalar values, preserving char
/// boundaries (never splits a multi-byte char). Returns `s` unchanged
/// when it is already `<= n` chars.
pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect()
    }
}

/// Issue #23: redact credentials embedded in URLs before persisting/logging
/// best-effort error and payload text. Covers the two common leak shapes:
///   * URL userinfo — `scheme://user:pass@host` → `scheme://REDACTED@host`
///   * secret-bearing query params — `?token=…`, `&api_key=…`, etc. → `…=REDACTED`
///
/// Conservative by design: it only rewrites these well-known shapes, so it can
/// never corrupt non-secret text. Used on error-path strings (rare), so the regex
/// compile cost is amortized via `OnceLock`.
pub fn scrub_secrets(input: &str) -> String {
    use std::sync::OnceLock;

    use regex::Regex;

    static USERINFO: OnceLock<Regex> = OnceLock::new();
    static QUERY_SECRET: OnceLock<Regex> = OnceLock::new();

    let userinfo = USERINFO.get_or_init(|| {
        // scheme://<userinfo>@  — userinfo has no '/', '@' or whitespace.
        Regex::new(r"([a-zA-Z][a-zA-Z0-9+.\-]*://)[^/@\s]+@").expect("valid userinfo regex")
    });
    let query_secret = QUERY_SECRET.get_or_init(|| {
        // ?key=value / &key=value for known-sensitive keys (case-insensitive).
        Regex::new(
            r"(?i)([?&](?:token|key|secret|password|passwd|access_token|api[_-]?key|auth|sig|signature)=)[^&\s#]+",
        )
        .expect("valid query-secret regex")
    });

    let step1 = userinfo.replace_all(input, "${1}REDACTED@");
    query_secret
        .replace_all(&step1, "${1}REDACTED")
        .into_owned()
}

#[cfg(test)]
mod scrub_tests {
    use super::scrub_secrets;

    #[test]
    fn redacts_url_userinfo() {
        assert_eq!(
            scrub_secrets("connect failed: https://alice:s3cret@hooks.example/x"),
            "connect failed: https://REDACTED@hooks.example/x"
        );
    }

    #[test]
    fn redacts_secret_query_params() {
        assert_eq!(
            scrub_secrets("GET https://api.example/v1?token=abc123&page=2"),
            "GET https://api.example/v1?token=REDACTED&page=2"
        );
        assert_eq!(
            scrub_secrets("https://h/x?api_key=KKK&access_token=TTT"),
            "https://h/x?api_key=REDACTED&access_token=REDACTED"
        );
    }

    #[test]
    fn leaves_non_secret_text_unchanged() {
        let s = "timeout after 15s talking to https://hooks.example/path?page=2";
        assert_eq!(scrub_secrets(s), s);
    }
}
