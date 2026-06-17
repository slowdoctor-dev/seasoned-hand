use crate::plan::{Phase, PhaseStatus, Plan};

/// Render the plan as a sticky-context-friendly string capped at
/// roughly `token_cap` tokens. Each phase title gets an initial budget
/// derived from `token_cap / phases × 3` (the `× 3` is a rough
/// chars-per-token heuristic for English text — close enough that the
/// `shrink_longest_title` loop below rarely runs more than 1-2 passes
/// to fit the cap). Minimum per-title budget is 8 chars so the marker
/// `[active]` still reads coherently.
pub fn sticky_render(plan: &Plan, token_cap: usize) -> String {
    let mut phases = plan.phases.clone();
    phases.sort_by_key(|p| p.id);
    let mut titles = phases.iter().map(|p| p.title.clone()).collect::<Vec<_>>();
    let initial_budget = token_cap
        .saturating_div(phases.len().max(1))
        .saturating_mul(3)
        .max(8);
    for title in &mut titles {
        truncate_title(title, initial_budget);
    }

    loop {
        let rendered = format_with_titles(&plan.goal, &phases, &titles);
        if estimate_tokens(&rendered) <= token_cap {
            return rendered;
        }
        if !shrink_longest_title(&mut titles) {
            return rendered;
        }
    }
}

fn format_with_titles(goal: &str, phases: &[Phase], titles: &[String]) -> String {
    let mut out = String::from("== PLAN ==\n");
    out.push_str(&format!("Goal: {goal}\n"));
    for (phase, title) in phases.iter().zip(titles.iter()) {
        let marker = match phase.status {
            PhaseStatus::Done => "[done]",
            PhaseStatus::Active => "[active]",
            PhaseStatus::Pending => "[pending]",
        };
        out.push_str(&format!("Phase {} {}: {}\n", phase.id, marker, title));
    }
    out.push_str("== END PLAN ==\n");
    out
}

fn shrink_longest_title(titles: &mut [String]) -> bool {
    let Some((idx, longest)) = titles
        .iter()
        .enumerate()
        .max_by_key(|(_, t)| t.chars().count())
    else {
        return false;
    };
    let len = longest.chars().count();
    if len <= 1 {
        return false;
    }
    if len == 2 {
        titles[idx] = "…".into();
        return true;
    }
    let mut out = String::new();
    for c in titles[idx].chars().take(len - 2) {
        out.push(c);
    }
    out.push('…');
    titles[idx] = out;
    true
}

fn truncate_title(title: &mut String, max_chars: usize) {
    if title.chars().count() <= max_chars {
        return;
    }
    let mut out = String::new();
    for c in title.chars().take(max_chars.saturating_sub(1)) {
        out.push(c);
    }
    out.push('…');
    *title = out;
}

pub fn estimate_tokens(input: &str) -> usize {
    // Issue #23: this runs every agent iteration (via `sticky_render`), where the
    // result gates a `<= token_cap` truncation loop. A tokenizer-init failure must
    // not panic the loop — but the fallback must FAIL CLOSED, i.e. never
    // *under*-estimate (an under-estimate would let an over-budget plan through).
    // Byte length is a guaranteed upper bound on the token count (every token spans
    // >= 1 byte), so it over-estimates and keeps the cap safe. It over-truncates
    // when the tokenizer is unavailable — acceptable for a rare cold-start failure.
    match tiktoken_rs::p50k_base() {
        Ok(bpe) => bpe.encode_ordinary(input).len(),
        Err(_) => input.len(),
    }
}
