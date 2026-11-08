//! Text helpers: sanitising untrusted case text and normalising policy text.
//!
//! Claim fields (diagnosis, procedure, hospital name) are untrusted input. They end
//! up in finding statements, and statements feed the optional rationale prompt, so
//! markup and control characters are stripped and length is capped. Decisions and
//! amounts are protected structurally elsewhere; this only narrows the attack surface.

/// Strip control characters, markup brackets and backticks, collapse whitespace, cap length.
pub fn clean(text: &str, cap: usize) -> String {
    let replaced: String = text
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | '`') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let collapsed = collapse_ws(&replaced);
    collapsed
        .chars()
        .take(cap)
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lowercase + collapsed whitespace, for phrase containment checks.
pub fn norm(text: &str) -> String {
    collapse_ws(&text.to_lowercase())
}

/// Truncate at a word boundary, appending " ..." when cut.
pub fn truncate_words(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let cut: String = text.chars().take(limit).collect();
    match cut.rfind(' ') {
        Some(pos) => format!("{} ...", &cut[..pos]),
        None => format!("{cut} ..."),
    }
}

/// Match a policy term at the start of a word in lowercased `text`
/// (so "cyst" matches "cysts" but not "cholecystectomy").
pub fn has_term(text: &str, term: &str) -> bool {
    let hay = text.to_lowercase();
    let needle = term.to_lowercase();
    if needle.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(pos) = hay[from..].find(&needle) {
        let abs = from + pos;
        let prev = hay[..abs].chars().next_back();
        if !prev.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
            return true;
        }
        from = abs + needle.len().max(1);
        if from >= hay.len() {
            break;
        }
        while !hay.is_char_boundary(from) {
            from += 1;
        }
    }
    false
}

/// Locate `needle` in `haystack` ignoring case and runs of whitespace; returns the
/// haystack's own wording for the matched span.
pub fn locate(needle: &str, haystack: &str) -> Option<String> {
    let tokens: Vec<String> = needle.split_whitespace().map(regex::escape).collect();
    if tokens.is_empty() {
        return None;
    }
    let pattern = format!("(?i){}", tokens.join(r"\s+"));
    let re = regex::Regex::new(&pattern).ok()?;
    re.find(haystack).map(|m| m.as_str().to_string())
}

/// Format an amount with thousands separators and no decimals ("20,000").
pub fn fmt_inr(v: f64) -> String {
    let rounded = v.round() as i64;
    let neg = rounded < 0;
    let digits = rounded.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if neg {
        format!("-{out}")
    } else {
        out
    }
}

/// Format a percentage without a trailing ".0" ("1", "0.5").
pub fn fmt_pct(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_strips_markup() {
        assert_eq!(
            clean("  <script>alert(1)</script>\n hi ", 200),
            "script alert(1) /script hi"
        );
        assert_eq!(clean("abcdef", 3), "abc");
    }

    #[test]
    fn has_term_word_start() {
        assert!(has_term("Ovarian cysts", "cyst"));
        assert!(!has_term("cholecystectomy", "cyst"));
        assert!(has_term("D&C procedure", "d&c"));
    }

    #[test]
    fn locate_is_whitespace_insensitive() {
        assert_eq!(
            locate("stone in  the", "Any Stone in\nthe biliary"),
            Some("Stone in\nthe".to_string())
        );
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_inr(1234567.4), "1,234,567");
        assert_eq!(fmt_inr(100.0), "100");
        assert_eq!(fmt_pct(1.0), "1");
        assert_eq!(fmt_pct(0.5), "0.5");
    }
}
