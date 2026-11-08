//! Numeric-threshold extraction from retrieved policy text.
//!
//! Admissibility logic must stay tied to what was actually retrieved, not to a
//! hardcoded lookup table that could drift from the source PDF. These helpers pull
//! the number a dimension needs (day count, month count, percentage) out of the
//! retrieved chunk. If the pattern is not found, the caller must treat the threshold
//! as unknown (INSUFFICIENT_EVIDENCE) and never guess.

use std::sync::LazyLock;

use regex::Regex;

fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid regex")
}

static WAIT_A: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)waiting period of (\d+)\s*days"));
static WAIT_B: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(\d+)\s*days?\s*waiting period"));
static PED_MONTHS: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(\d+)\s*months of continuous coverage"));
static YEARS: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(\d+)\s*year"));
static PRE_WIN: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)maximum of (\d+)\s*days? immediately preceding"));
static POST_WIN: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)maximum of (\d+)\s*days? immediately following"));
static CLAUSE_START: LazyLock<Regex> = LazyLock::new(|| re(r"\b\d{1,2}\.\s|NB\d+:|\b[a-c]\)\s"));
static PERCENT: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)([\d.]+)\s*%\s*(?:of\s*)?(?:the\s*)?(?:Basic\s*)?Sum\s*(?:Insured|Assured)")
});
static FLAT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(?:Rupees|Rs\.?)\s*(\d+)"));

fn first_int(re: &Regex, text: &str) -> Option<i64> {
    re.captures(text)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

pub fn extract_waiting_days(text: &str) -> Option<i64> {
    first_int(&WAIT_A, text).or_else(|| first_int(&WAIT_B, text))
}

pub fn extract_pre_existing_months(text: &str) -> Option<i64> {
    first_int(&PED_MONTHS, text)
}

pub fn extract_years(text: &str) -> Option<i64> {
    first_int(&YEARS, text)
}

pub fn extract_pre_post_windows(text: &str) -> (Option<i64>, Option<i64>) {
    (first_int(&PRE_WIN, text), first_int(&POST_WIN, text))
}

/// Split policy text into clause-level segments (numbered items, NB notes,
/// a)/b)/c) sub-points). Limits are stated inside one clause, so binding a keyword to
/// a number within the same clause is more reliable than a character window.
pub fn clauses(text: &str) -> Vec<&str> {
    let mut starts: Vec<usize> = CLAUSE_START.find_iter(text).map(|m| m.start()).collect();
    if starts.first() != Some(&0) {
        starts.insert(0, 0);
    }
    let mut out = vec![];
    for (i, &s) in starts.iter().enumerate() {
        let e = starts.get(i + 1).copied().unwrap_or(text.len());
        let piece = &text[s..e];
        if !piece.trim().is_empty() {
            out.push(piece);
        }
    }
    out
}

fn percent_in(clause: &str) -> Option<f64> {
    PERCENT
        .captures_iter(clause)
        .filter_map(|c| {
            c.get(1)
                .and_then(|m| m.as_str().trim_matches('.').parse::<f64>().ok())
        })
        .next()
}

fn flat_in(clause: &str) -> Option<f64> {
    FLAT.captures(clause)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

/// The first clause that mentions `near` and states a limit.
pub fn clause_with<'a>(text: &'a str, near: &str) -> Option<&'a str> {
    let near = near.to_lowercase();
    clauses(text).into_iter().find(|c| {
        c.to_lowercase().contains(&near) && (percent_in(c).is_some() || flat_in(c).is_some())
    })
}

/// The `<pct>% of ... Sum Insured/Assured` figure in the clause mentioning `near`.
pub fn extract_percent_of_si(text: &str, near: &str) -> Option<f64> {
    clause_with(text, near).and_then(percent_in)
}

/// A flat rupee cap ("Rupees 1000/-") in the clause mentioning `near`.
pub fn extract_flat_amount(text: &str, near: &str) -> Option<f64> {
    clause_with(text, near).and_then(flat_in)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_days() {
        assert_eq!(
            extract_waiting_days("subject to a waiting period of 30 days from"),
            Some(30)
        );
        assert_eq!(extract_waiting_days("a 30 day waiting period"), Some(30));
        assert_eq!(extract_waiting_days("nothing here"), None);
    }

    #[test]
    fn clause_binding() {
        let text = "a) Room, Boarding expenses up to 1% of Sum Insured per day b) Ambulance charges up to 1% of Basic Sum Insured or Rupees 1000/- whichever is less";
        assert_eq!(extract_percent_of_si(text, "Room"), Some(1.0));
        assert_eq!(extract_flat_amount(text, "Ambulance"), Some(1000.0));
        assert_eq!(extract_flat_amount(text, "Room"), None);
    }

    #[test]
    fn windows() {
        let t = "for a maximum of 30 days immediately preceding ... for a maximum of 60 days immediately following";
        assert_eq!(extract_pre_post_windows(t), (Some(30), Some(60)));
    }
}
