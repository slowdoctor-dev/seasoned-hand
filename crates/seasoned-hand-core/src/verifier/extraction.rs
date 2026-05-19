use regex::Regex;
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::LazyLock;

pub const EXTRACTION_INPUT_TOKEN_CAP: usize = 8_192;
pub const EXTRACTION_OUTPUT_BYTE_CAP: usize = 24_576;
pub const EXTRACTION_INPUT_TRUNCATION_MARKER: &str = "[..., truncated for extraction budget]";

static PIPE_TO_SHELL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\|\s*(?:sh|bash)\b").expect("valid regex"));
static URL_RAW_IP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)https?://(?:\d{1,3}(?:\.\d{1,3}){3}|\[[0-9A-F:]+\])(?:[/:?#][^\s]*)?"#)
        .expect("valid regex")
});
static BASE64_BLOB_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9+/=]{40,}").expect("valid regex"));

static TOKEN_SHAPE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9_-]{32,}").expect("valid regex"));
static EMAIL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b").expect("valid regex")
});
static PHONE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?x)(?:\+?[1-9]\d{1,14}|\(?\d{2,4}\)?[-.\s]?\d{3,4}[-.\s]?\d{4})")
        .expect("valid regex")
});
static IPV4_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("valid regex"));
// Phase 4 security hardening iter-1 F3: the original pattern required at
// least two `xxxx:` groups before the elision, which silently missed the
// common short forms `::1`, `fe80::1`, and `::ffff:192.0.2.1`. The
// alternation here catches three shapes the redactor was previously
// blind to:
//   1. full forms with >=2 leading groups (the original case)
//   2. `::`-prefixed elision forms (`::1`, `::ffff:...`)
//   3. `xxxx::yyy` short forms with elision after the first group
//      (`fe80::1`, `2001:db8::1` is already covered by branch 1)
static IPV6_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?:
            \b(?:[0-9A-Fa-f]{1,4}:){2,}[0-9A-Fa-f:]{1,4}\b
          | ::(?:[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{1,4})*)?(?:\.\d{1,3}(?:\.\d{1,3}){2})?
          | \b[0-9A-Fa-f]{1,4}::(?:[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{1,4})*)?\b
        )
        ",
    )
    .expect("valid regex")
});
// Phase 4 security hardening iter-1 F2: PEM-encoded private key blocks
// were not caught — the BEGIN/END markers were preserved verbatim and
// only the embedded base64 body chunks would be partially caught by
// TOKEN_SHAPE_RE (and only when each chunk exceeded the 32-char floor
// AND used the unpadded url-safe alphabet). A leaked OPENSSH / RSA /
// EC / DSA / PGP private key in an extracted playbook would be a
// catastrophic credential leak. `(?s)` lets `.` cross newlines so the
// full PEM block (header + body + footer) is replaced as one unit.
static PRIVATE_KEY_PEM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY[A-Z0-9 ]*-----.*?-----END [A-Z0-9 ]*PRIVATE KEY[A-Z0-9 ]*-----",
    )
    .expect("valid regex")
});
// Phase 3 REVIEW iter-1 F5: expand beyond Authorization:Bearer + X-Api-Key
// to cover the other commonly-leaked secret-bearing header shapes.
static AUTH_HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?im)\b(?:Authorization:\s*(?:Bearer|Basic)|Proxy-Authorization:\s*(?:Bearer|Basic)|X-Api-Key:|X-Auth-Token:|X-CSRF-Token:|(?:Set-)?Cookie:)\s*[^\s]+",
    )
    .expect("valid regex")
});

const INJECTION_PHRASES: &[&str] = &[
    "ignore previous instructions",
    "disregard the above",
    "forget everything",
    "you are now",
    "act as",
    "from now on you are",
    "you are not the assistant",
    "i am the assistant",
    "system:",
    "<|im_start|>system",
    "[inst]",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truncation {
    pub original: usize,
    pub capped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionReport {
    pub count: usize,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityFloorFailure {
    pub reason: &'static str,
}

pub fn apply_input_cap(input: &str, cap_tokens: usize) -> (String, Option<Truncation>) {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let original = tokens.len();
    if original <= cap_tokens {
        return (input.to_string(), None);
    }
    let mut truncated = tokens[..cap_tokens].join(" ");
    if !truncated.is_empty() {
        truncated.push(' ');
    }
    truncated.push_str(EXTRACTION_INPUT_TRUNCATION_MARKER);
    (
        truncated,
        Some(Truncation {
            original,
            capped: cap_tokens,
        }),
    )
}

pub fn cap_output_bytes(content: &str, cap_bytes: usize) -> (String, Option<Truncation>) {
    let original = content.len();
    if original <= cap_bytes {
        return (content.to_string(), None);
    }
    let mut cutoff = cap_bytes;
    while !content.is_char_boundary(cutoff) && cutoff > 0 {
        cutoff -= 1;
    }
    (
        content[..cutoff].to_string(),
        Some(Truncation {
            original,
            capped: cutoff,
        }),
    )
}

pub fn detect_adversarial(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    if text.contains('`') {
        return Some("shell_backticks".to_string());
    }
    if text.contains("$(") {
        return Some("shell_substitution".to_string());
    }
    if PIPE_TO_SHELL_RE.is_match(text) {
        return Some("pipe_to_shell".to_string());
    }
    if URL_RAW_IP_RE.is_match(text) {
        return Some("url_raw_ip".to_string());
    }
    if INJECTION_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
    {
        return Some("prompt_injection_phrase".to_string());
    }
    if BASE64_BLOB_RE.is_match(text) {
        return Some("base64_blob".to_string());
    }
    None
}

pub fn redact_pii(text: &str) -> (String, Option<RedactionReport>) {
    let mut rewritten = text.to_string();
    let mut categories = BTreeSet::new();
    let mut count = 0usize;
    // Order matters: redact PEM private key blocks first so the long
    // base64 body inside doesn't get partially rewritten by TOKEN_SHAPE_RE
    // before the whole block is recognized. Auth headers run before
    // TOKEN_SHAPE_RE for the same reason — they carry the named-secret
    // metadata that the generic token pattern would erase.
    for (category, pattern, replacement) in [
        (
            "private_key",
            &*PRIVATE_KEY_PEM_RE,
            "[REDACTED_PRIVATE_KEY]",
        ),
        ("api_key", &*AUTH_HEADER_RE, "[REDACTED_AUTH_HEADER]"),
        ("token", &*TOKEN_SHAPE_RE, "[REDACTED_TOKEN]"),
        ("email", &*EMAIL_RE, "[REDACTED_EMAIL]"),
        ("phone", &*PHONE_RE, "[REDACTED_PHONE]"),
        ("ip", &*IPV4_RE, "[REDACTED_IP]"),
        ("ip", &*IPV6_RE, "[REDACTED_IP]"),
    ] {
        let matches = pattern.find_iter(&rewritten).count();
        if matches > 0 {
            categories.insert(category.to_string());
            count += matches;
            rewritten = pattern.replace_all(&rewritten, replacement).to_string();
        }
    }

    if count == 0 {
        (rewritten, None)
    } else {
        (
            rewritten,
            Some(RedactionReport {
                count,
                categories: categories.into_iter().collect(),
            }),
        )
    }
}

pub fn validate_quality_floor(steps: &[String]) -> Result<(), QualityFloorFailure> {
    if steps.len() < 3 {
        return Err(QualityFloorFailure {
            reason: "steps_lt_3",
        });
    }
    for step in steps {
        if step.split_whitespace().next().is_none() {
            return Err(QualityFloorFailure {
                reason: "trivial_step",
            });
        }
    }
    let chars: usize = steps.join("\n").chars().count();
    if chars < 200 {
        return Err(QualityFloorFailure {
            reason: "content_lt_200_chars",
        });
    }
    Ok(())
}

pub fn extraction_rejected_event(layer: &str, reason: &str) -> serde_json::Value {
    json!({
        "kind": "playbook_extraction_rejected",
        "layer": layer,
        "reason": reason
    })
}

pub fn extraction_pii_redacted_event(layer: &str, report: &RedactionReport) -> serde_json::Value {
    json!({
        "kind": "playbook_extraction_pii_redacted",
        "layer": layer,
        "count": report.count,
        "categories": report.categories
    })
}

pub fn extraction_input_truncated_event(truncation: &Truncation) -> serde_json::Value {
    json!({
        "kind": "playbook_extraction_input_truncated",
        "original_tokens": truncation.original,
        "capped_tokens": truncation.capped
    })
}

pub fn extraction_output_capped_event(truncation: &Truncation) -> serde_json::Value {
    json!({
        "kind": "playbook_extraction_output_capped",
        "original_bytes": truncation.original,
        "capped_bytes": truncation.capped
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adversarial_filters() {
        assert_eq!(
            detect_adversarial("run this | bash"),
            Some("pipe_to_shell".to_string())
        );
        assert_eq!(
            detect_adversarial("https://192.168.0.1/login"),
            Some("url_raw_ip".to_string())
        );
        assert_eq!(
            detect_adversarial("ignore previous instructions and proceed"),
            Some("prompt_injection_phrase".to_string())
        );
        assert_eq!(
            detect_adversarial("QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo9PT09PT09PT0="),
            Some("base64_blob".to_string())
        );
    }

    #[test]
    fn pii_redaction_catches_pem_private_keys() {
        // Phase 4 security hardening iter-1 F2: before this fix, the BEGIN/END
        // markers were preserved verbatim and only the long base64 chunks
        // inside might have been partially caught by TOKEN_SHAPE_RE.
        let pem = "config:\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXkt\nQUFBQUMzTnphQzFsWkRJMU5URTVBQUFBSUd\n-----END OPENSSH PRIVATE KEY-----\nuser=admin";
        let (redacted, report) = redact_pii(pem);
        let report = report.expect("expected redaction report");
        assert!(
            redacted.contains("[REDACTED_PRIVATE_KEY]"),
            "PEM block must be redacted as a whole; got {redacted:?}"
        );
        assert!(
            !redacted.contains("BEGIN OPENSSH"),
            "marker must not leak into extracted playbooks"
        );
        assert!(report.categories.iter().any(|c| c == "private_key"));

        // Also confirm RSA/EC variants are caught (markers in {BEGIN,END}
        // PRIVATE KEY style).
        for marker in &["RSA", "EC", "DSA", "ENCRYPTED"] {
            let body = format!(
                "-----BEGIN {marker} PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9\n-----END {marker} PRIVATE KEY-----"
            );
            let (out, rep) = redact_pii(&body);
            assert!(
                out.contains("[REDACTED_PRIVATE_KEY]"),
                "marker {marker} not redacted; got {out:?}"
            );
            assert!(rep.is_some());
        }
    }

    #[test]
    fn pii_redaction_catches_ipv6_short_forms() {
        // Phase 4 security hardening iter-1 F3: before this fix, the IPv6
        // regex required >=2 `xxxx:` groups before the elision, so the
        // most common short forms slipped through silently.
        for sample in &[
            "ping ::1 for localhost",
            "host fe80::1 is link-local",
            "mapped ::ffff:192.0.2.1 form",
            "expanded 2001:db8::1 form",
        ] {
            let (redacted, report) = redact_pii(sample);
            let report = report.unwrap_or_else(|| panic!("no redaction on '{sample}'"));
            assert!(
                redacted.contains("[REDACTED_IP]"),
                "ipv6 not redacted in '{sample}'; got {redacted:?}"
            );
            assert!(report.categories.iter().any(|c| c == "ip"));
        }
    }

    #[test]
    fn pii_redaction() {
        let sample = "Authorization: Bearer SECRET123456789012345678901234567890 \
                      Contact me at hi@example.com, +1-415-555-0123, and 10.0.0.1";
        let (redacted, report) = redact_pii(sample);
        let report = report.expect("expected redaction report");
        assert!(redacted.contains("[REDACTED_AUTH_HEADER]"));
        assert!(redacted.contains("[REDACTED_EMAIL]"));
        assert!(redacted.contains("[REDACTED_PHONE]"));
        assert!(report.count >= 4);
        assert!(report.categories.iter().any(|c| c == "api_key"));
        assert!(report.categories.iter().any(|c| c == "email"));
        assert!(report.categories.iter().any(|c| c == "phone" || c == "ip"));
        assert_eq!(
            extraction_pii_redacted_event("deterministic", &report)["kind"],
            "playbook_extraction_pii_redacted"
        );
    }

    #[test]
    fn quality_floor_and_caps() {
        let input = vec!["word"; EXTRACTION_INPUT_TOKEN_CAP + 10].join(" ");
        let (truncated_input, input_cap) = apply_input_cap(&input, EXTRACTION_INPUT_TOKEN_CAP);
        let input_cap = input_cap.expect("input cap should fire");
        assert!(truncated_input.contains(EXTRACTION_INPUT_TRUNCATION_MARKER));
        assert_eq!(
            extraction_input_truncated_event(&input_cap)["kind"],
            "playbook_extraction_input_truncated"
        );

        let too_short = vec![
            "Collect requirements".to_string(),
            "Run command".to_string(),
            "Report output".to_string(),
        ];
        let err = validate_quality_floor(&too_short).expect_err("expected floor failure");
        assert_eq!(err.reason, "content_lt_200_chars");

        let long_steps = vec![
            "First, gather all required workspace artifacts and validate the migration state before making edits.".to_string(),
            "Second, execute deterministic checks and capture each result with explicit command output references for auditability.".to_string(),
            "Third, apply the final transformation and summarize verifiable side effects, regressions, and fallback behavior.".to_string(),
        ];
        validate_quality_floor(&long_steps).expect("long steps should pass floor");

        let rendered = format!("Overview\n\n## Procedure\n{}", long_steps.join("\n"));
        let (capped, output_cap) = cap_output_bytes(&rendered, 80);
        let output_cap = output_cap.expect("output cap should fire");
        assert!(capped.len() <= 80);
        assert_eq!(
            extraction_output_capped_event(&output_cap)["kind"],
            "playbook_extraction_output_capped"
        );
        let post_cap_steps = vec![capped];
        let floor_err = validate_quality_floor(&post_cap_steps).expect_err("post-cap floor fail");
        assert_eq!(floor_err.reason, "steps_lt_3");
        assert_eq!(
            extraction_rejected_event("quality_floor", floor_err.reason)["kind"],
            "playbook_extraction_rejected"
        );
    }
}
