//! Tokenisation and a light suffix stemmer shared by every retrieval stage.

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "of", "to", "in", "on", "for", "by", "with", "is", "are", "be",
    "been", "as", "at", "it", "its", "this", "that", "these", "those", "from", "any", "all",
    "such", "which", "who", "will", "shall", "may", "under", "than", "not", "no", "if", "into",
    "per", "up", "we", "our", "you", "your", "has", "have", "had", "was", "were", "there", "their",
    "them", "they", "also", "other", "can", "out", "do", "does",
];

/// `[a-z0-9]+` tokens, lowercased.
pub fn tokenize(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut out = vec![];
    let mut cur = String::new();
    for ch in lower.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            cur.push(ch);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Conservative suffix stripping: folds plural / -ing / -ed / -isation variants
/// ("hospitalisation", "hospitalization", "hospital") onto one stem.
pub fn stem(word: &str) -> String {
    if word.len() <= 3 || word.chars().all(|c| c.is_ascii_digit()) {
        return word.to_string();
    }
    let w = word;
    if let Some(base) = w.strip_suffix("es") {
        // "classes" -> "class", "boxes" -> "box"; otherwise only the "s" goes ("diseases" -> "disease").
        if base.len() >= 3
            && (base.ends_with("ch")
                || base.ends_with("sh")
                || base.ends_with('x')
                || base.ends_with("ss")
                || base.ends_with('z'))
        {
            return base.to_string();
        }
    }
    for (suffix, repl) in [
        ("isations", ""),
        ("izations", ""),
        ("isation", ""),
        ("ization", ""),
        ("ations", "ate"),
        ("ation", "ate"),
        ("ments", ""),
        ("ment", ""),
        ("ings", ""),
        ("ing", ""),
        ("ies", "y"),
        ("edly", ""),
        ("ed", ""),
        ("s", ""),
    ] {
        if let Some(base) = w.strip_suffix(suffix) {
            let blocked = suffix == "s"
                && (base.ends_with('s') || base.ends_with('u') || base.ends_with('i'));
            if base.len() >= 3 && !blocked {
                return format!("{base}{repl}");
            }
        }
    }
    w.to_string()
}

/// Tokens without stopwords, stemmed.
pub fn tokens_stemmed(text: &str) -> Vec<String> {
    tokenize(text)
        .into_iter()
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .map(|t| stem(&t))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_fold_variants() {
        assert_eq!(stem("hospitalisation"), stem("hospitalization"));
        assert_eq!(stem("days"), "day");
        assert_eq!(stem("diseases"), stem("disease"));
        assert_eq!(stem("30"), "30");
        assert_eq!(stem("illness"), "illness");
    }
}
