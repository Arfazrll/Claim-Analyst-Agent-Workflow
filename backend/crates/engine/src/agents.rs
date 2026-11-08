//! Policy Evidence, Coverage & Exclusion and Validation agents.

use std::collections::BTreeMap;
use std::time::Instant;

use crate::dimensions::{by_key, UNMODELLED};
use crate::interpretation;
use crate::llm::LlmClient;
use crate::model::{
    CaseState, Decision, FindingStatus, HandoffKind, ValidationOutcome, ValidationStatus, Verdict,
};
use crate::retrieval::{HybridRetriever, TopK};
use crate::verification::verify_state;

/// Policy Evidence Agent: hybrid retrieval + rerank per applicable dimension.
/// On a validation retry the candidate pool and hand-over size are doubled.
pub fn policy_evidence(state: &mut CaseState, retriever: &HybridRetriever, top_k: TopK) {
    let started = Instant::now();
    let factor = if state.widen_retrieval { 2 } else { 1 };
    let k = top_k.widened(factor);
    let mut total = 0;
    for key in state.dimensions.clone() {
        let Some(dim) = by_key(&key) else { continue };
        let query = (dim.query)(&state.facts);
        let results = retriever.search(&query, k);
        total += results.len();
        state.evidence_by_dimension.insert(key, results);
    }
    let mode = if factor > 1 {
        format!("widened x{factor} (retry)")
    } else {
        "standard".to_string()
    };
    let n = state.dimensions.len();
    state.log(
        "PolicyEvidenceAgent",
        "hybrid_retrieve_and_rerank",
        format!("{mode}: ran {n} queries, {total} evidence chunks returned after fusion+rerank."),
        Some(total),
        started,
        &["facts", "dimensions"],
        &["evidence_by_dimension"],
    );
    state.hand_off(
        "PolicyEvidenceAgent",
        "CoverageExclusionAgent",
        format!("{total} ranked evidence chunks with page/section/chunk metadata"),
        HandoffKind::Forward,
    );
}

/// Coverage & Exclusion Agent: rule-based assessment of every dimension, then the
/// optional (one-directional) LLM interpretation pass.
pub async fn coverage_exclusion(
    state: &mut CaseState,
    llm: &dyn LlmClient,
    interpret: bool,
    retriever: &HybridRetriever,
) {
    let started = Instant::now();
    // The unknown-dimension check runs last: it only looks at clauses no modelled
    // dimension has already used.
    let mut ordered: Vec<String> = state
        .dimensions
        .iter()
        .filter(|k| *k != UNMODELLED)
        .cloned()
        .collect();
    ordered.extend(
        state
            .dimensions
            .iter()
            .filter(|k| *k == UNMODELLED)
            .cloned(),
    );
    for key in ordered {
        if key == UNMODELLED {
            let mut cited: Vec<String> = state
                .findings
                .iter()
                .flat_map(|f| f.citations.iter().map(|c| c.chunk_id.clone()))
                .collect();
            cited.sort();
            cited.dedup();
            state.facts.cited_chunk_ids = cited;
        }
        let Some(dim) = by_key(&key) else { continue };
        let evidence = state
            .evidence_by_dimension
            .get(&key)
            .cloned()
            .unwrap_or_default();
        let (finding, limits, missing) = (dim.evaluate)(&state.facts, &evidence);
        state.findings.push(finding);
        state.applicable_limits.extend(limits);
        state.missing_fields.extend(missing);
    }
    if interpret {
        interpretation::run(state, llm, retriever).await;
    }
    let count = |s: FindingStatus| state.findings.iter().filter(|f| f.status == s).count();
    let (ex, lim, ins) = (
        count(FindingStatus::SupportsExclusion),
        count(FindingStatus::SupportsLimit),
        count(FindingStatus::InsufficientEvidence),
    );
    let llm_detail = if !state.llm_interpretation.enabled {
        String::new()
    } else if let Some(e) = &state.llm_interpretation.error {
        format!(" LLM interpretation: {e}.")
    } else {
        format!(
            " LLM interpretation: {} verified observation(s) accepted, {} rejected.",
            state.llm_interpretation.accepted.len(),
            state.llm_interpretation.rejected.len()
        )
    };
    let reads: &[&str] = if state.llm_interpretation.enabled {
        &["facts", "evidence_by_dimension", "llm"]
    } else {
        &["facts", "evidence_by_dimension"]
    };
    let detail = format!(
        "{} findings: {ex} exclusion-supporting, {lim} limit-supporting, {ins} insufficient-evidence; {} limit(s) computed.{llm_detail}",
        state.findings.len(),
        state.applicable_limits.len()
    );
    state.log(
        "CoverageExclusionAgent",
        "assess_dimensions",
        detail,
        None,
        started,
        reads,
        &["findings", "applicable_limits", "missing_fields"],
    );
    let payload = format!(
        "{} cited findings, {} limit(s), {} gap(s)",
        state.findings.len(),
        state.applicable_limits.len(),
        state.missing_fields.len()
    );
    state.hand_off(
        "CoverageExclusionAgent",
        "DecisionAgent",
        payload,
        HandoffKind::Forward,
    );
}

/// Validation Agent: verifies every claim against its cited chunk. On a failed final
/// attempt the decision is downgraded to NEEDS_REVIEW (abstain rather than present
/// something the evidence check could not confirm).
pub fn validation(state: &mut CaseState, corpus: &BTreeMap<String, String>, final_attempt: bool) {
    let started = Instant::now();
    let rows = verify_state(state, corpus);
    let mut counts: BTreeMap<String, usize> = ["SUPPORTED", "UNSUPPORTED", "CONTRADICTED"]
        .iter()
        .map(|k| (k.to_string(), 0))
        .collect();
    for r in &rows {
        let key = match r.verdict {
            Verdict::Supported => "SUPPORTED",
            Verdict::Unsupported => "UNSUPPORTED",
            Verdict::Contradicted => "CONTRADICTED",
        };
        *counts.get_mut(key).expect("key") += 1;
    }
    let problems: Vec<_> = rows
        .iter()
        .filter(|r| r.verdict != Verdict::Supported)
        .collect();
    let unsupported: Vec<String> = problems
        .iter()
        .map(|r| {
            format!(
                "[{}] {:?}: {} ({}) - {}",
                r.dimension,
                r.verdict,
                r.claim,
                r.chunk_id.as_deref().unwrap_or("no chunk"),
                r.reason
            )
        })
        .collect();
    let status = if problems.is_empty() {
        ValidationStatus::Pass
    } else {
        ValidationStatus::Fail
    };
    let n_problems = problems.len();
    let total = rows.len();
    let supported = counts["SUPPORTED"];
    state.validation = Some(ValidationOutcome {
        status,
        unsupported_claims: unsupported,
        notes: format!("{supported}/{total} claims verified against retrieved policy text."),
        verifications: rows,
        counts: counts.clone(),
        attempts: state.attempt,
    });
    let downgrade = status == ValidationStatus::Fail
        && final_attempt
        && state.decision != Some(Decision::NeedsReview);
    if downgrade {
        state.decision = Some(Decision::NeedsReview);
        state.confidence = state.confidence.min(0.3);
        state.rationale.push_str(&format!(
            "\n\nDowngraded to NEEDS_REVIEW: evidence verification could not confirm {n_problems} claim(s) even after a widened re-retrieval."
        ));
    }
    let writes: &[&str] = if status == ValidationStatus::Fail && final_attempt {
        &["validation", "decision"]
    } else {
        &["validation"]
    };
    state.log(
        "ValidationAgent",
        "verify_claims_against_evidence",
        format!(
            "status={}; {supported} supported, {} unsupported, {} contradicted of {total} claims",
            if status == ValidationStatus::Pass {
                "PASS"
            } else {
                "FAIL"
            },
            counts["UNSUPPORTED"],
            counts["CONTRADICTED"]
        ),
        None,
        started,
        &[
            "findings",
            "applicable_limits",
            "decision",
            "evidence_by_dimension",
        ],
        writes,
    );
}
