//! Evidence verification: does the cited chunk actually say what the claim says?
//!
//! `decision claim -> citation -> retrieved chunk -> evidence check -> verdict`
//!
//! * SUPPORTED    the cited chunk contains what the claim asserts
//! * UNSUPPORTED  it does not (or the chunk was never retrieved)
//! * CONTRADICTED the chunk states a different figure, the claim's own arithmetic
//!   fails, or the decision conflicts with its own findings
//!
//! Deterministic: no LLM involved, so results are reproducible and cheap.

use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;

use crate::model::{
    Assertion, CaseState, CheckResult, ClaimKind, ClaimVerification, Decision, FindingStatus,
    Verdict,
};
use crate::rules as rx;
use crate::text_safety::{fmt_inr, norm, truncate_words};

fn unit_stem(unit: &str) -> &str {
    match unit {
        "days" | "day" => "day",
        "months" | "month" => "month",
        "years" | "year" => "year",
        other => other,
    }
}

fn numbers_with_unit(text: &str, unit: &str) -> BTreeSet<i64> {
    let pattern = format!(r"(?i)(\d+)\s*-?\s*{}s?\b", regex::escape(unit_stem(unit)));
    let re = Regex::new(&pattern).expect("regex");
    re.captures_iter(text)
        .filter_map(|c| c.get(1)?.as_str().parse().ok())
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 0.5_f64.max(1e-6 * a.abs().max(b.abs()))
}

type Res = Vec<(Verdict, CheckResult)>;

fn phrase_check(phrase: &str, normed: &str) -> (Verdict, CheckResult) {
    let ok = normed.contains(&norm(phrase));
    (
        if ok {
            Verdict::Supported
        } else {
            Verdict::Unsupported
        },
        CheckResult::new(
            format!("chunk contains '{phrase}'"),
            ok,
            if ok { "" } else { "phrase not present" },
        ),
    )
}

fn check_threshold(values: &[crate::model::ValueUnit], must: &[String], text: &str) -> Res {
    let mut out = vec![];
    for item in values {
        let found = numbers_with_unit(text, &item.unit);
        let name = format!("chunk states {} {}", item.value, item.unit);
        if found.contains(&item.value) {
            out.push((
                Verdict::Supported,
                CheckResult::new(
                    name,
                    true,
                    format!("found {} {} in cited text", item.value, item.unit),
                ),
            ));
        } else if !found.is_empty() {
            out.push((
                Verdict::Contradicted,
                CheckResult::new(
                    name,
                    false,
                    format!(
                        "claim says {} {} but the chunk only states {:?} {}",
                        item.value, item.unit, found, item.unit
                    ),
                ),
            ));
        } else {
            out.push((
                Verdict::Unsupported,
                CheckResult::new(
                    name,
                    false,
                    format!("no '{}' figure in the cited chunk", item.unit),
                ),
            ));
        }
    }
    let normed = norm(text);
    out.extend(must.iter().map(|p| phrase_check(p, &normed)));
    out
}

fn check_phrases(must: &[String], any_of: &[Vec<String>], text: &str) -> Res {
    let normed = norm(text);
    let mut out: Res = must.iter().map(|p| phrase_check(p, &normed)).collect();
    for group in any_of {
        let ok = group.iter().any(|p| normed.contains(&norm(p)));
        out.push((
            if ok {
                Verdict::Supported
            } else {
                Verdict::Unsupported
            },
            CheckResult::new(
                format!("chunk contains one of {group:?}"),
                ok,
                if ok { "" } else { "none present" },
            ),
        ));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn check_limit(
    keyword: &str,
    percent: Option<f64>,
    flat: Option<f64>,
    sum_insured: Option<f64>,
    multiplier: f64,
    claimed: Option<f64>,
    cap: Option<f64>,
    deduction: Option<f64>,
    text: &str,
) -> Res {
    let mut out = vec![];
    let clause = rx::clause_with(text, keyword);
    let stated_pct = rx::extract_percent_of_si(text, keyword);
    let stated_flat = rx::extract_flat_amount(text, keyword);

    if let Some(p) = percent {
        let name = format!("clause for '{keyword}' states {p}% of Sum Insured");
        out.push(match stated_pct {
            None => (
                Verdict::Unsupported,
                CheckResult::new(
                    name,
                    false,
                    format!("no percentage limit found near '{keyword}' in the chunk"),
                ),
            ),
            Some(s) if close(s, p) => (
                Verdict::Supported,
                CheckResult::new(name, true, format!("chunk states {s}%")),
            ),
            Some(s) => (
                Verdict::Contradicted,
                CheckResult::new(name, false, format!("chunk states {s}%, claim uses {p}%")),
            ),
        });
    }
    if let Some(fl) = flat {
        let name = format!("clause for '{keyword}' states flat cap INR {}", fmt_inr(fl));
        out.push(match stated_flat {
            None => (
                Verdict::Unsupported,
                CheckResult::new(name, false, "no flat rupee cap found in the clause"),
            ),
            Some(s) if close(s, fl) => (
                Verdict::Supported,
                CheckResult::new(name, true, format!("chunk states INR {}", fmt_inr(s))),
            ),
            Some(s) => (
                Verdict::Contradicted,
                CheckResult::new(
                    name,
                    false,
                    format!(
                        "chunk states INR {}, claim uses INR {}",
                        fmt_inr(s),
                        fmt_inr(fl)
                    ),
                ),
            ),
        });
    }
    if let (Some(si), Some(c)) = (sum_insured, cap) {
        let candidates: Vec<f64> = [percent.map(|p| p / 100.0 * si * multiplier), flat]
            .into_iter()
            .flatten()
            .collect();
        let expected = if candidates.is_empty() {
            c
        } else {
            candidates.into_iter().fold(f64::INFINITY, f64::min)
        };
        let ok = close(expected, c);
        out.push((
            if ok {
                Verdict::Supported
            } else {
                Verdict::Contradicted
            },
            CheckResult::new(
                "cap = limit x sum insured (x days)",
                ok,
                format!(
                    "recomputed cap INR {} vs claimed INR {}",
                    fmt_inr(expected),
                    fmt_inr(c)
                ),
            ),
        ));
    }
    if let (Some(cl), Some(c), Some(d)) = (claimed, cap, deduction) {
        let expected = (cl - c).max(0.0);
        let ok = close(expected, d);
        out.push((
            if ok {
                Verdict::Supported
            } else {
                Verdict::Contradicted
            },
            CheckResult::new(
                "deduction = claimed - cap",
                ok,
                format!(
                    "INR {} - INR {} = INR {} vs reported INR {}",
                    fmt_inr(cl),
                    fmt_inr(c),
                    fmt_inr(expected),
                    fmt_inr(d)
                ),
            ),
        ));
    }
    if clause.is_none() && out.is_empty() {
        out.push((
            Verdict::Unsupported,
            CheckResult::new(
                "limit clause located",
                false,
                format!("no clause mentions '{keyword}'"),
            ),
        ));
    }
    out
}

fn check_absence(
    term: &str,
    allowed: &[String],
    text: &str,
    corpus: &BTreeMap<String, String>,
    cited: &str,
) -> Res {
    let term = term.to_lowercase();
    let in_cited = text.to_lowercase().contains(&term);
    let mut out = vec![(
        if in_cited {
            Verdict::Supported
        } else {
            Verdict::Unsupported
        },
        CheckResult::new(
            format!("cited chunk mentions '{term}'"),
            in_cited,
            if in_cited {
                ""
            } else {
                "term absent from the cited chunk"
            },
        ),
    )];
    let allowed: BTreeSet<&str> = allowed
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(cited))
        .collect();
    let elsewhere: Vec<&String> = corpus
        .iter()
        .filter(|(cid, body)| {
            !allowed.contains(cid.as_str()) && body.to_lowercase().contains(&term)
        })
        .map(|(cid, _)| cid)
        .collect();
    let ok = elsewhere.is_empty();
    out.push((
        if ok {
            Verdict::Supported
        } else {
            Verdict::Contradicted
        },
        CheckResult::new(
            format!("'{term}' appears nowhere else in the policy"),
            ok,
            if ok {
                "scanned the whole policy: only the cited definition mentions it".to_string()
            } else {
                format!(
                    "also mentioned in {elsewhere:?}, so 'no explicit clause' is not established"
                )
            },
        ),
    ));
    out
}

fn combine(results: Res) -> (Verdict, Vec<CheckResult>) {
    let verdict = if results.iter().any(|(v, _)| *v == Verdict::Contradicted) {
        Verdict::Contradicted
    } else if results.iter().any(|(v, _)| *v == Verdict::Unsupported) {
        Verdict::Unsupported
    } else {
        Verdict::Supported
    };
    (verdict, results.into_iter().map(|(_, c)| c).collect())
}

pub fn verify_assertion(
    assertion: Option<&Assertion>,
    text: &str,
    corpus: &BTreeMap<String, String>,
    chunk_id: &str,
) -> (Verdict, String, Vec<CheckResult>) {
    let Some(a) = assertion else {
        return (
            Verdict::Unsupported,
            "citation carries no checkable assertion".into(),
            vec![],
        );
    };
    let results = match a {
        Assertion::Threshold {
            values,
            must_contain,
        } => check_threshold(values, must_contain, text),
        Assertion::Phrases {
            must_contain,
            any_of,
        } => check_phrases(must_contain, any_of, text),
        Assertion::Limit {
            keyword,
            percent,
            flat,
            sum_insured,
            multiplier,
            claimed,
            cap,
            deduction,
        } => check_limit(
            keyword,
            *percent,
            *flat,
            *sum_insured,
            *multiplier,
            *claimed,
            *cap,
            *deduction,
            text,
        ),
        Assertion::Absence {
            term,
            allowed_chunk_ids,
        } => check_absence(term, allowed_chunk_ids, text, corpus, chunk_id),
        Assertion::Quote { quote } => {
            let ok = !quote.trim().is_empty() && norm(text).contains(&norm(quote));
            vec![(
                if ok {
                    Verdict::Supported
                } else {
                    Verdict::Unsupported
                },
                CheckResult::new(
                    "quoted text appears verbatim in the cited chunk",
                    ok,
                    if ok {
                        ""
                    } else {
                        "the quote is not in the cited chunk"
                    },
                ),
            )]
        }
    };
    if results.is_empty() {
        return (
            Verdict::Unsupported,
            "assertion produced no checks".into(),
            vec![],
        );
    }
    let (verdict, checks) = combine(results);
    let reason = if verdict == Verdict::Supported {
        "all evidence checks passed".to_string()
    } else {
        checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| {
                if c.detail.is_empty() {
                    c.name.clone()
                } else {
                    c.detail.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    (verdict, reason, checks)
}

fn excerpt(text: &str, assertion: Option<&Assertion>) -> String {
    if let Some(kw) = assertion.and_then(Assertion::keyword) {
        if let Some(clause) = rx::clause_with(text, kw) {
            return clause.trim().chars().take(260).collect();
        }
    }
    truncate_words(text, 260)
}

pub fn verify_state(
    state: &CaseState,
    corpus: &BTreeMap<String, String>,
) -> Vec<ClaimVerification> {
    let mut rows: Vec<ClaimVerification> = vec![];
    let mut n = 0usize;

    let mut add = |rows: &mut Vec<ClaimVerification>,
                   kind: ClaimKind,
                   dimension: &str,
                   c: &crate::model::Citation| {
        n += 1;
        let claim_id = format!("C{n}");
        match state.lookup_chunk(&c.chunk_id) {
            None => rows.push(ClaimVerification {
                claim_id,
                kind,
                dimension: dimension.into(),
                claim: c.claim.clone(),
                chunk_id: Some(c.chunk_id.clone()),
                page: Some(c.page),
                section: Some(c.section.clone()),
                verdict: Verdict::Unsupported,
                reason: "cited chunk was never retrieved for this case".into(),
                checks: vec![CheckResult::new(
                    "cited chunk was retrieved",
                    false,
                    "citation points outside the retrieved evidence",
                )],
                excerpt: None,
            }),
            Some(ev) => {
                let (verdict, reason, mut checks) =
                    verify_assertion(c.assertion.as_ref(), &ev.text, corpus, &ev.chunk_id);
                checks.insert(
                    0,
                    CheckResult::new(
                        "cited chunk was retrieved",
                        true,
                        format!("{}, page {}", ev.section, ev.page_start),
                    ),
                );
                rows.push(ClaimVerification {
                    claim_id,
                    kind,
                    dimension: dimension.into(),
                    claim: c.claim.clone(),
                    chunk_id: Some(ev.chunk_id.clone()),
                    page: Some(ev.page_start),
                    section: Some(ev.section.clone()),
                    verdict,
                    reason,
                    checks,
                    excerpt: Some(excerpt(&ev.text, c.assertion.as_ref())),
                });
            }
        }
    };

    for f in state.findings.iter().filter(|f| f.applicable) {
        for c in &f.citations {
            add(&mut rows, ClaimKind::PolicyClaim, &f.dimension, c);
        }
    }
    for l in &state.applicable_limits {
        for c in &l.citations {
            add(&mut rows, ClaimKind::LimitClaim, &l.dimension, c);
        }
    }
    let decision_row = verify_decision(state, &rows, rows.len());
    rows.push(decision_row);
    rows
}

/// Does the final decision follow from findings whose evidence checks passed?
fn verify_decision(
    state: &CaseState,
    rows: &[ClaimVerification],
    start: usize,
) -> ClaimVerification {
    let supported_dims: BTreeSet<&str> = rows
        .iter()
        .filter(|r| r.verdict == Verdict::Supported && r.kind == ClaimKind::PolicyClaim)
        .map(|r| r.dimension.as_str())
        .collect();
    let supported_limits = rows
        .iter()
        .any(|r| r.verdict == Verdict::Supported && r.kind == ClaimKind::LimitClaim);
    let exclusions: Vec<_> = state
        .findings
        .iter()
        .filter(|f| f.status == FindingStatus::SupportsExclusion)
        .collect();
    let insufficient = state
        .findings
        .iter()
        .any(|f| f.status == FindingStatus::InsufficientEvidence);
    let unknowns = !state.facts.explicit_unknowns.is_empty();
    let mut checks = vec![];
    let mut verdict = Verdict::Supported;
    let mut reason = "decision follows from verified findings".to_string();

    match state.decision {
        Some(Decision::NotAdmissible) => {
            let ok = exclusions
                .iter()
                .any(|f| supported_dims.contains(f.dimension.as_str()));
            checks.push(CheckResult::new(
                "a verified exclusion finding backs the rejection",
                ok,
                "",
            ));
            if !ok {
                verdict = Verdict::Unsupported;
                reason = "rejection is not backed by a verified exclusion citation".into();
            }
        }
        Some(Decision::AdmissibleWithLimits) | Some(Decision::PartiallyAdmissible) => {
            checks.push(CheckResult::new(
                "a verified limit/deduction backs the reduced payout",
                supported_limits,
                "",
            ));
            if !supported_limits {
                verdict = Verdict::Unsupported;
                reason = "limit-based decision has no verified limit citation".into();
            }
        }
        Some(Decision::Admissible) => {
            let deductions = state
                .applicable_limits
                .iter()
                .any(|l| l.deduction_inr.unwrap_or(0.0) > 0.0);
            checks.push(CheckResult::new(
                "no exclusion finding contradicts admissibility",
                exclusions.is_empty(),
                "",
            ));
            checks.push(CheckResult::new(
                "no unresolved finding or open question remains",
                !insufficient && !unknowns,
                "",
            ));
            checks.push(CheckResult::new(
                "no limit reduces the payable amount",
                !deductions,
                "",
            ));
            if !exclusions.is_empty() || insufficient || unknowns || deductions {
                verdict = Verdict::Contradicted;
                reason = "an exclusion, unresolved question or limit exists but the claim was called fully admissible".into();
            }
        }
        Some(Decision::NeedsReview) => {
            let has_reason = insufficient
                || unknowns
                || state
                    .missing_fields
                    .iter()
                    .any(|m| m.field == "policy_start_date/claim_date");
            checks.push(CheckResult::new(
                "a stated reason justifies abstaining",
                has_reason,
                "",
            ));
            if !has_reason {
                verdict = Verdict::Unsupported;
                reason = "abstained without a recorded unresolved finding".into();
            }
        }
        None => {
            verdict = Verdict::Unsupported;
            reason = "no decision was produced".into();
        }
    }
    ClaimVerification {
        claim_id: format!("C{}", start + 1),
        kind: ClaimKind::DecisionClaim,
        dimension: "decision".into(),
        claim: format!(
            "Decision is {}",
            state.decision.map(|d| d.as_str()).unwrap_or("NONE")
        ),
        chunk_id: None,
        page: None,
        section: None,
        verdict,
        reason,
        checks,
        excerpt: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ValueUnit;

    #[test]
    fn threshold_detects_contradiction() {
        let a = Assertion::Threshold {
            values: vec![ValueUnit {
                value: 45,
                unit: "days".into(),
            }],
            must_contain: vec![],
        };
        let (v, _, _) = verify_assertion(
            Some(&a),
            "a waiting period of 30 days applies",
            &BTreeMap::new(),
            "x",
        );
        assert_eq!(v, Verdict::Contradicted);
        let ok = Assertion::Threshold {
            values: vec![ValueUnit {
                value: 30,
                unit: "days".into(),
            }],
            must_contain: vec![],
        };
        assert_eq!(
            verify_assertion(
                Some(&ok),
                "a waiting period of 30 days applies",
                &BTreeMap::new(),
                "x"
            )
            .0,
            Verdict::Supported
        );
    }

    #[test]
    fn limit_arithmetic_is_checked() {
        let text = "a) Room, Boarding and Nursing expenses up to 1% of Sum Insured";
        let good = Assertion::Limit {
            keyword: "Room".into(),
            percent: Some(1.0),
            flat: None,
            sum_insured: Some(500000.0),
            multiplier: 4.0,
            claimed: Some(30000.0),
            cap: Some(20000.0),
            deduction: Some(10000.0),
        };
        assert_eq!(
            verify_assertion(Some(&good), text, &BTreeMap::new(), "x").0,
            Verdict::Supported
        );
        let tampered = Assertion::Limit {
            keyword: "Room".into(),
            percent: Some(1.0),
            flat: None,
            sum_insured: Some(500000.0),
            multiplier: 4.0,
            claimed: Some(30000.0),
            cap: Some(20000.0),
            deduction: Some(5000.0),
        };
        assert_eq!(
            verify_assertion(Some(&tampered), text, &BTreeMap::new(), "x").0,
            Verdict::Contradicted
        );
    }

    #[test]
    fn absence_scans_corpus() {
        let mut corpus = BTreeMap::new();
        corpus.insert("a".to_string(), "Experimental treatment means".to_string());
        corpus.insert("b".to_string(), "unrelated".to_string());
        let a = Assertion::Absence {
            term: "experimental".into(),
            allowed_chunk_ids: vec!["a".into()],
        };
        assert_eq!(
            verify_assertion(Some(&a), "Experimental treatment means", &corpus, "a").0,
            Verdict::Supported
        );
        corpus.insert("c".to_string(), "excludes experimental drugs".to_string());
        assert_eq!(
            verify_assertion(Some(&a), "Experimental treatment means", &corpus, "a").0,
            Verdict::Contradicted
        );
    }
}
