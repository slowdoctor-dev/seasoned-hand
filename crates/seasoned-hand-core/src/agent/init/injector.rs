use crate::matcher::MatchedPlaybook;

pub const INJECTION_BYTE_CAP: usize = 12_288;
const TRUNCATION_MARKER: &str = "\n...[truncated for playbook injection cap]";

#[derive(Debug, Clone)]
pub struct InjectionResult {
    pub rendered_prefix: String,
    pub injected_ids: Vec<String>,
    pub total_bytes: usize,
    pub truncated: bool,
    pub original_bytes: usize,
    pub matched_count: usize,
}

pub fn build_injection(matches: &[MatchedPlaybook], byte_cap: usize) -> Option<InjectionResult> {
    if matches.is_empty() {
        return None;
    }
    let mut segments = Vec::new();
    let mut ids = Vec::new();
    for (idx, m) in matches.iter().take(3).enumerate() {
        ids.push(m.playbook_id.clone());
        segments.push(format!(
            "### Playbook {}: {}\n{}\n",
            idx + 1,
            m.title,
            m.content
        ));
    }
    let full = format!(
        "Use these previously verified playbooks if relevant:\n\n{}",
        segments.join("\n")
    );
    let original_bytes = full.len();
    if original_bytes <= byte_cap {
        let matched_count = ids.len();
        return Some(InjectionResult {
            rendered_prefix: full,
            injected_ids: ids,
            total_bytes: original_bytes,
            truncated: false,
            original_bytes,
            matched_count,
        });
    }

    let marker_bytes = TRUNCATION_MARKER.len();
    let mut cutoff = byte_cap.saturating_sub(marker_bytes);
    while cutoff > 0 && !full.is_char_boundary(cutoff) {
        cutoff -= 1;
    }
    let mut rendered = full[..cutoff].to_string();
    rendered.push_str(TRUNCATION_MARKER);
    Some(InjectionResult {
        rendered_prefix: rendered,
        injected_ids: ids,
        total_bytes: byte_cap,
        truncated: true,
        original_bytes,
        matched_count: matches.len().min(3),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::MatcherMode;

    fn m(id: &str, title: &str, content: &str) -> MatchedPlaybook {
        MatchedPlaybook {
            playbook_id: id.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            content_excerpt: content.chars().take(32).collect::<String>(),
            matcher_mode: MatcherMode::Production,
            match_score: 2.0,
            success_count: 0,
            failure_count: 0,
        }
    }

    #[test]
    fn top3_behavior() {
        assert!(build_injection(&[], INJECTION_BYTE_CAP).is_none());

        let one = vec![m("pb-1", "One", "Do one")];
        let out1 = build_injection(&one, INJECTION_BYTE_CAP).expect("one");
        assert_eq!(out1.injected_ids, vec!["pb-1"]);

        let two = vec![m("pb-1", "One", "Do one"), m("pb-2", "Two", "Do two")];
        let out2 = build_injection(&two, INJECTION_BYTE_CAP).expect("two");
        assert_eq!(out2.injected_ids, vec!["pb-1", "pb-2"]);

        let four = vec![
            m("pb-1", "One", "Do one"),
            m("pb-2", "Two", "Do two"),
            m("pb-3", "Three", "Do three"),
            m("pb-4", "Four", "Do four"),
        ];
        let out3 = build_injection(&four, INJECTION_BYTE_CAP).expect("three only");
        assert_eq!(out3.injected_ids, vec!["pb-1", "pb-2", "pb-3"]);
    }

    #[test]
    fn byte_cap_and_event() {
        let long = "x".repeat(20_000);
        let matches = vec![m("pb-1", "One", &long)];
        let out = build_injection(&matches, 512).expect("cap");
        assert!(out.truncated);
        assert_eq!(out.total_bytes, 512);
        assert!(out.original_bytes > out.total_bytes);
        assert!(out.rendered_prefix.ends_with(TRUNCATION_MARKER));
    }
}
