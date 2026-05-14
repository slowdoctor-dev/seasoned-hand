//! Sender allow-list for [`super::EmailChannel`].
//!
//! `INTAKE_EMAIL_ALLOWED_SENDERS` env is a comma-separated list of
//! regex patterns; each is anchored with `(?i)\A...\z` so callers don't
//! have to remember to do it themselves. An empty allow-list rejects
//! every sender (default-deny per architecture §9 / phase-2/DEBT.md #4).
//!
//! refs: /specs/phase-2/architecture.md §9 "Email intake authentication"

use regex::Regex;

use crate::channel::ChannelError;

/// Compiled allow-list of sender patterns.
#[derive(Default, Clone, Debug)]
pub struct AllowList {
    patterns: Vec<Regex>,
}

impl AllowList {
    /// Build from a comma-separated env-shaped string. Whitespace
    /// around commas is trimmed; empty entries are skipped (so
    /// `"a,,b"` is treated as `["a", "b"]`).
    pub fn parse(raw: &str) -> Result<Self, ChannelError> {
        let mut patterns = Vec::new();
        for entry in raw.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            // Anchor + case-insensitive so `me@example.com` literally
            // matches the whole address. Operators wanting partial /
            // wildcard matches use `.*` explicitly inside the entry.
            let anchored = format!("(?i)\\A{entry}\\z");
            let re = Regex::new(&anchored).map_err(|err| {
                ChannelError::Internal(format!(
                    "email: invalid allow-list pattern {entry:?}: {err}"
                ))
            })?;
            patterns.push(re);
        }
        Ok(Self { patterns })
    }

    /// `true` iff at least one pattern matches the sender. Empty
    /// allow-list always returns `false` — default-deny.
    pub fn allows(&self, sender: &str) -> bool {
        self.patterns.iter().any(|re| re.is_match(sender))
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_allowlist_rejects_everything() {
        let al = AllowList::parse("").unwrap();
        assert!(al.is_empty());
        assert!(!al.allows("anyone@example.com"));
        assert!(!al.allows(""));
    }

    #[test]
    fn literal_address_matches_case_insensitively() {
        let al = AllowList::parse("me@example.com").unwrap();
        assert!(al.allows("me@example.com"));
        assert!(al.allows("ME@Example.COM"));
        assert!(!al.allows("other@example.com"));
        // Substring must NOT match (anchored).
        assert!(!al.allows("not-me@example.com"));
        assert!(!al.allows("me@example.com.evil.test"));
    }

    #[test]
    fn wildcard_pattern_works() {
        let al = AllowList::parse(r".*@example\.com").unwrap();
        assert!(al.allows("a@example.com"));
        assert!(al.allows("b@example.com"));
        assert!(!al.allows("a@other.com"));
    }

    #[test]
    fn invalid_regex_is_rejected() {
        let err = AllowList::parse("[unclosed").expect_err("must reject");
        match err {
            ChannelError::Internal(msg) => assert!(msg.contains("invalid allow-list pattern")),
            other => panic!("unexpected variant: {other:?}"),
        }
    }
}
