pub use crate::time::{now_micros, now_micros_str};

pub const MAX_PROGRESS_LINE_CHARS: usize = 200;

pub fn truncate_line(line: &str) -> String {
    let chars = line.chars().count();
    if chars <= MAX_PROGRESS_LINE_CHARS {
        return line.to_string();
    }
    let mut out = String::new();
    for c in line.chars().take(MAX_PROGRESS_LINE_CHARS.saturating_sub(1)) {
        out.push(c);
    }
    out.push('…');
    out
}

pub fn append_line(existing: &str, line: &str) -> String {
    let line = truncate_line(line);
    let mut out = String::with_capacity(existing.len() + line.len() + 64);
    out.push_str(existing);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("{}  user           {}\n", now_micros_str(), line));
    out
}

pub fn tail_lines(content: &str, lines: usize) -> String {
    let lines = lines.max(1);
    let all = content.lines().collect::<Vec<_>>();
    let start = all.len().saturating_sub(lines);
    let mut out = all[start..].join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}
