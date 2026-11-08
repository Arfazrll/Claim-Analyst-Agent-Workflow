//! Decision Agent: combines specialist findings into the final decision.
//!
//! The combination is a small, auditable precedence rule over already-cited findings.
//! An LLM may only rephrase the result, and its text is rejected if it is
//! disproportionate, does not name the decision, argues against a non-approving
//! decision, or introduces any number the evidence does not contain.

use std::collections::BTreeSet;
use std::sync::LazyLock;
use std::time::Instant;

use regex::Regex;

use crate::llm::LlmClient;
use crate::model::{CaseState, Decision, FindingStatus, HandoffKind};

const WHOLE_CLAIM_EXCLUSIONS: &[&str] = &[
    "initial_waiting_period",
    "named_disease_first_year_waiting_period",
    "pre_existing_disease_waiting_period",
    "domiciliary_treatment_conditions",
    "cosmetic_exclusion",
];

static OVERRIDE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(overrid\w*|overrul\w*|pre-?approved|disregard\w*|ignore (?:the|all|any|previous)|paid in full|must be paid|should be approved)\b")
        .expect("regex")
});
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+(?:\.\d+)?").expect("regex"));

fn norm_number(n: &str) -> String {
    let n = if n.contains('.') {
        n.trim_end_matches('0').trim_end_matches('.')
    } else {
        n
    };
    let t = n.trim_start_matches('0');
    if t.is_empty() || t.starts_with('.') {
        format!("0{t}")
    } else {
        t.to_string()
    }
}

pub fn numbers(text: &str) -> BTreeSet<String> {
    NUMBER
        .find_iter(&text.replace(',', ""))
        .map(|m| norm_number(m.as_str()))
        .collect()
}

fn human(d: Decision) -> String {
    d.as_str().to_lowercase().replace('_', " ")
}

/// Accept LLM prose only if it is proportionate, names the decision, and adds no number.
pub fn guard_llm_rationale(state: &CaseState, text: Option<&str>) -> Result<String, String> {
    let Some(text) = text.map(str::trim).filter(|t| !t.is_empty()) else {
        return Err("no LLM text".into());
    };
    let decision = state.decision.unwrap_or(Decision::NeedsReview);
    if text.len() >= 4 * state.rationale.len() + 200 {
        return Err("LLM text disproportionately long".into());
    }
    if !text
        .to_lowercase()
        .replace('_', " ")
        .contains(&human(decision))
    {
        return Err("LLM text does not name the decision".into());
    }
    if !decision.as_str().starts_with("ADMISSIBLE") && OVERRIDE.is_match(text) {
        return Err("LLM text argues against the decision".into());
    }
    let amounts = serde_json::to_string(&state.amounts(decision)).unwrap_or_default();
    let limits: Vec<&str> = state
        .applicable_limits
        .iter()
        .map(|l| l.description.as_str())
        .collect();
    let allowed = numbers(&format!(
        "{} {} {} {}",
        state.rationale,
        state.raw_case,
        limits.join(" "),
        amounts
    ));
    let invented: Vec<String> = numbers(text)
        .difference(&allowed)
        .take(5)
        .cloned()
        .collect();
    if !invented.is_empty() {
        return Err(format!(
            "LLM text contains figures not in the evidence: {}",
            invented.join(", ")
        ));
    }
    Ok(text.to_string())
}

fn template_rationale(state: &CaseState) -> String {
    let lines: Vec<String> = state
        .findings
        .iter()
        .filter(|f| f.applicable)
        .map(|f| format!("- [{}] {}", f.dimension, f.statement))
        .collect();
    format!(
        "Decision {} for case {}, based on:\n{}",
        state.decision.unwrap_or(Decision::NeedsReview),
        state.case_id,
        lines.join("\n")
    )
}

pub fn decide(state: &mut CaseState) -> (usize, usize, usize, usize) {
    let insufficient = state
        .findings
        .iter()
        .filter(|f| f.status == FindingStatus::InsufficientEvidence)
        .count();
    let exclusions = state
        .findings
        .iter()
        .filter(|f| {
            f.status == FindingStatus::SupportsExclusion
                && WHOLE_CLAIM_EXCLUSIONS.contains(&f.dimension.as_str())
        })
        .count();
    let window_limit = state.findings.iter().any(|f| {
        f.status == FindingStatus::SupportsLimit && f.dimension == "pre_post_hospitalization_window"
    });
    let sub_limits = state
        .applicable_limits
        .iter()
        .filter(|l| l.dimension == "category_sub_limits" && l.deduction_inr.unwrap_or(0.0) > 0.0)
        .count();
    let window_deductions = state
        .applicable_limits
        .iter()
        .filter(|l| {
            l.dimension == "pre_post_hospitalization_window" && l.deduction_inr.unwrap_or(0.0) > 0.0
        })
        .count();
    let missing_dates = state
        .missing_fields
        .iter()
        .any(|m| m.field == "policy_start_date/claim_date");
    let unresolved = !state.facts.explicit_unknowns.is_empty() || insufficient > 0;

    // Precedence: without dates nothing can be decided; one verified ground for
    // rejection is enough (unresolved side-questions cannot rescue an excluded
    // claim); only then does unresolved evidence force a review.
    let (decision, confidence) = if missing_dates {
        (Decision::NeedsReview, 0.25)
    } else if exclusions > 0 {
        (
            Decision::NotAdmissible,
            if unresolved { 0.75 } else { 0.85 },
        )
    } else if unresolved {
        (
            Decision::NeedsReview,
            (0.4 - 0.03 * insufficient as f64).max(0.25),
        )
    } else if window_limit && window_deductions > 0 {
        (Decision::PartiallyAdmissible, 0.75)
    } else if sub_limits > 0 {
        (Decision::AdmissibleWithLimits, 0.8)
    } else {
        (Decision::Admissible, 0.85)
    };
    state.decision = Some(decision);
    state.confidence = confidence;
    state.rationale = template_rationale(state);
    (insufficient, exclusions, window_deductions, sub_limits)
}

pub async fn run(state: &mut CaseState, llm: &dyn LlmClient) {
    let started = Instant::now();
    let (insufficient, exclusions, windows, subs) = decide(state);
    let decision = state.decision.unwrap_or(Decision::NeedsReview);

    let prompt = format!(
        "Case {}. Decision: {}.\nFindings (each already evidence-cited -- do not add new facts):\n{}\n\nRewrite as one concise reviewer-facing paragraph that states the decision ({}) explicitly.",
        state.case_id,
        decision,
        state.rationale,
        human(decision)
    );
    let reply = llm.rationale(&prompt).await;
    match guard_llm_rationale(state, reply.as_deref()) {
        Ok(text) => {
            state.rationale = text;
            state.rationale_source = "llm".into();
        }
        Err(why) if reply.is_some() => {
            state.rationale_source = format!("template (LLM text rejected: {why})")
        }
        Err(_) => state.rationale_source = "template".into(),
    }

    state.log(
        "DecisionAgent",
        "combine_findings",
        format!(
            "decision={decision} confidence={} (insufficient={insufficient}, exclusions={exclusions}, time_window_limits={windows}, sub_limits={subs})",
            state.confidence
        ),
        None,
        started,
        &["findings", "applicable_limits", "missing_fields"],
        &["decision", "confidence", "rationale"],
    );
    state.hand_off(
        "DecisionAgent",
        "ValidationAgent",
        format!("decision {decision} to verify against its citations"),
        HandoffKind::Forward,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_normalisation() {
        let n = numbers("INR 10,000.00 and 0.50 and 007");
        assert!(n.contains("10000") && n.contains("0.5") && n.contains("7"));
    }
}
