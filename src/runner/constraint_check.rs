/// Returns the list of constraints whose keywords appear in `response`.
///
/// Heuristic only — a positive flag means the response is *worth a closer
/// look*, not that it definitely violates the constraint.
pub fn check_constraint_violations(response: &str, constraints: &[String]) -> Vec<String> {
    constraints
        .iter()
        .filter(|c| response_violates(response, c))
        .cloned()
        .collect()
}

/// Decide whether `response` appears to violate a single constraint.
///
/// Strategy: lowercase-compare and look for the strongest noun in the
/// constraint text, plus a small set of common code-level patterns.
pub fn response_violates(response: &str, constraint: &str) -> bool {
    let r = response.to_lowercase();
    let c = constraint.to_lowercase();

    // Pattern-based triggers — same examples called out in the workflow doc.
    if c.contains("nightly") && r.contains("#![feature(") {
        return true;
    }
    if c.contains("tokio") && (r.contains("tokio::") || r.contains("use tokio")) {
        return true;
    }
    if c.contains("unsafe") && r.contains("unsafe ") {
        return true;
    }
    if c.contains("public api") && r.contains("breaking change") {
        return true;
    }

    // Generic fallback: pull keywords out of the constraint after stripping
    // negation phrasing like "do not", "never", "no". If any such keyword
    // appears in the response, flag it.
    for keyword in extract_keywords(&c) {
        if keyword.len() >= 4 && r.contains(&keyword) {
            return true;
        }
    }
    false
}

/// Pull candidate trigger words out of a constraint string.
fn extract_keywords(constraint: &str) -> Vec<String> {
    let stop_words = [
        "do", "not", "never", "no", "the", "a", "an", "and", "or", "to", "of", "for", "in", "on",
        "must", "should", "is", "be", "use", "with", "any", "this", "that", "these", "those",
    ];

    constraint
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .filter(|w| !stop_words.contains(&w.as_str()))
        .collect()
}
