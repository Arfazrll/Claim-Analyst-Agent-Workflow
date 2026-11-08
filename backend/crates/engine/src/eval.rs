//! Reproducible evaluation: hand-labelled expected outcomes, decision / amount
//! metrics, retrieval gold checks and verifier fault-injection controls.
//!
//! Labels were derived by reading each case's facts against the governing policy
//! clauses (e.g. room 1% of SI per day, practitioner fees 25%, medicines 40%,
//! ambulance "lower of 1% or Rs 1000"). They are a reading of the policy, not an
//! insurer's adjudication.

use std::collections::BTreeMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::OfflineLlm;
use crate::model::{Assertion, CaseState, Decision, Verdict};
use crate::verification::verify_state;
use crate::Engine;

pub const PUBLIC_CASES: &str = include_str!("../../../data/cases/public_cases.json");
pub const CUSTOM_CASES: &str = include_str!("../../../data/cases/custom_cases.json");
pub const ADVERSARIAL_CASES: &str = include_str!("../../../data/cases/adversarial_cases.json");

#[derive(Debug, Clone, Serialize)]
pub struct Expected {
    pub decision: Decision,
    pub primary_dimensions: &'static [&'static str],
    pub rationale: &'static str,
    pub deduction_inr: Option<f64>,
}

const fn e(
    decision: Decision,
    dims: &'static [&'static str],
    why: &'static str,
    ded: Option<f64>,
) -> Expected {
    Expected {
        decision,
        primary_dimensions: dims,
        rationale: why,
        deduction_inr: ded,
    }
}

use Decision::*;

pub static EXPECTED: &[(&str, Expected)] = &[
    ("PUB-001", e(AdmissibleWithLimits, &["category_sub_limits"], "Room 30,000 vs 1% x 500,000 x 4 days = 20,000 (-10,000); ambulance 1,200 vs lower of 1% and Rs 1,000 (-200).", Some(10200.0))),
    ("PUB-002", e(NotAdmissible, &["initial_waiting_period"], "19 days after inception, inside the 30-day initial waiting period, no continuity exception.", Some(0.0))),
    ("PUB-003", e(NotAdmissible, &["pre_existing_disease_waiting_period"], "27 months of continuous coverage is short of the 48-month pre-existing-disease wait.", Some(0.0))),
    ("PUB-004", e(AdmissibleWithLimits, &["domiciliary_treatment_conditions", "category_sub_limits"], "Room unavailability satisfies Domiciliary Treatment; 105,000 exceeds the 20% aggregate cap of 100,000 (-5,000).", Some(5000.0))),
    ("PUB-005", e(Admissible, &["day_care_less_than_24h"], "Cataract/eye surgery is a named <24h day-care procedure; no cap exceeded.", Some(0.0))),
    ("PUB-006", e(NeedsReview, &["hospital_definition"], "hospital_registered and medical_necessity_confirmed are explicitly unresolved (null).", None)),
    ("PUB-007", e(AdmissibleWithLimits, &["category_sub_limits"], "SI 1,000,000, 4 days: room -40,000; fees -50,000; medicines -100,000; ambulance -500.", Some(190500.0))),
    ("PUB-008", e(NotAdmissible, &["cosmetic_exclusion"], "Cosmetic surgery with no injury/disease basis is excluded.", Some(0.0))),
    ("PUB-009", e(Admissible, &["pre_post_hospitalization_window"], "30-day pre / 60-day post windows satisfied and same condition confirmed.", Some(0.0))),
    ("PUB-010", e(Admissible, &["named_disease_first_year_waiting_period"], "First-year cataract waived by 1 prior-insurer year with history received.", Some(0.0))),
    ("PUB-011", e(NeedsReview, &["hospital_definition"], "Non-network facility with undocumented registration and minimum criteria.", None)),
    ("PUB-012", e(NeedsReview, &["experimental_unproven_treatment"], "Experimental treatment is defined but no clause unambiguously excludes it.", None)),
    ("CUST-001", e(NotAdmissible, &["named_disease_first_year_waiting_period"], "Hernia in the first policy year with no prior-insurer waiver.", Some(0.0))),
    ("CUST-002", e(Admissible, &["named_disease_first_year_waiting_period"], "Hysterectomy in the first year, waived by 2 prior-insurer years.", Some(0.0))),
    ("CUST-003", e(PartiallyAdmissible, &["pre_post_hospitalization_window", "category_sub_limits"], "Pre-hospitalisation 45 days before admission (-12,000); room -5,000.", Some(17000.0))),
    ("CUST-004", e(AdmissibleWithLimits, &["category_sub_limits"], "SI 300,000, 4 days: room 20,000 vs 12,000 (-8,000); irrelevant attributes ignored.", Some(8000.0))),
    ("CUST-005", e(NeedsReview, &["experimental_unproven_treatment"], "Experimental stem-cell therapy: same basis as PUB-012.", None)),
    ("CUST-006", e(NeedsReview, &["domiciliary_treatment_conditions", "unmodelled_policy_risk"], "Domiciliary conditions unstated; asthma named in an unmodelled exclusion.", None)),
    ("CUST-007", e(Admissible, &["pre_existing_disease_waiting_period"], "14 months + 4 prior-insurer years = 62 >= 48 months.", Some(0.0))),
    ("CUST-008", e(NeedsReview, &["unmodelled_policy_risk"], "Dental treatment named in an unmodelled exclusion.", None)),
    ("CUST-009", e(NeedsReview, &["unmodelled_policy_risk"], "Pregnancy/childbirth named in an unmodelled exclusion.", None)),
    ("CUST-010", e(NeedsReview, &["day_care_less_than_24h"], "12-hour stay for a procedure not on the named list; no justification.", None)),
    ("CUST-011", e(NotAdmissible, &["initial_waiting_period", "cosmetic_exclusion"], "14 days after inception and a cosmetic procedure.", Some(0.0))),
    ("CUST-012", e(NotAdmissible, &["cosmetic_exclusion"], "Cosmetic exclusion established; unresolved registration cannot rescue it.", Some(0.0))),
    ("CUST-013", e(AdmissibleWithLimits, &["category_sub_limits"], "SI 200,000, 2 days: room -8,000; fees -10,000; medicines -10,000; ambulance -500.", Some(28500.0))),
    ("CUST-014", e(NotAdmissible, &["named_disease_first_year_waiting_period"], "First-year cataract; prior history not received, so no reduction.", Some(0.0))),
    // Paraphrase cases: the diagnosis never uses the policy's own term. The clinical
    // synonym lexicon links them to the first-year list, and a synonym match can only
    // hold a claim for review (a reviewer may well conclude NOT_ADMISSIBLE).
    ("ADV-001", e(NeedsReview, &["named_disease_first_year_waiting_period"], "'Clouding of the crystalline body' / phacoemulsification describe cataract, inside the first-year wait.", None)),
    ("ADV-002", e(NeedsReview, &["named_disease_first_year_waiting_period"], "Gallstones are a stone in the biliary system, inside the first-year wait.", None)),
];

pub fn expected(case_id: &str) -> Option<&'static Expected> {
    EXPECTED
        .iter()
        .find(|(id, _)| *id == case_id)
        .map(|(_, e)| e)
}

/// Every bundled case (public + custom + adversarial), in file order.
pub fn bundled_cases() -> Vec<Value> {
    [PUBLIC_CASES, CUSTOM_CASES, ADVERSARIAL_CASES]
        .iter()
        .flat_map(|s| serde_json::from_str::<Vec<Value>>(s).expect("bundled cases are valid JSON"))
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseResult {
    pub case_id: String,
    pub expected: Option<Decision>,
    pub actual: Decision,
    pub correct: Option<bool>,
    pub unsafe_decision: bool,
    pub expected_deduction_inr: Option<f64>,
    pub actual_deduction_inr: f64,
    pub amount_match: Option<bool>,
    pub claims_supported: usize,
    pub claims_total: usize,
    pub attempts: u32,
    pub latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalReport {
    pub engine_version: String,
    pub labelled_cases: usize,
    pub correct: usize,
    pub accuracy: f64,
    pub unsafe_decisions: usize,
    pub required_abstentions: usize,
    pub abstentions_caught: usize,
    pub amount_cases: usize,
    pub amount_exact: usize,
    pub claims_verified: usize,
    pub claims_total: usize,
    pub faults_injected: usize,
    pub faults_detected: usize,
    pub deterministic: bool,
    pub latency_p50_ms: f64,
    pub latency_p95_ms: f64,
    /// confusion[expected][actual] = count
    pub confusion: BTreeMap<String, BTreeMap<String, usize>>,
    pub per_class: BTreeMap<String, ClassMetrics>,
    pub cases: Vec<CaseResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClassMetrics {
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    pub support: usize,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx]
}

/// Verifier power: inject faults into a finished state and count how many the
/// evidence verifier catches (wrong numbers, tampered arithmetic, wrong chunks,
/// flipped decisions).
pub fn fault_injection(state: &CaseState, engine: &Engine) -> (usize, usize) {
    let corpus = engine.retriever().corpus();
    let mut injected = 0;
    let mut detected = 0;
    let caught = |s: &CaseState, dim: &str| {
        verify_state(s, &corpus)
            .iter()
            .any(|r| r.dimension == dim && r.verdict != Verdict::Supported)
    };

    for (fi, f) in state.findings.iter().enumerate() {
        if !f.applicable {
            continue;
        }
        for (ci, c) in f.citations.iter().enumerate() {
            if let Some(Assertion::Threshold { values, .. }) = &c.assertion {
                if !values.is_empty() {
                    let mut s = state.clone();
                    if let Some(Assertion::Threshold { values, .. }) =
                        &mut s.findings[fi].citations[ci].assertion
                    {
                        values[0].value += 7;
                    }
                    injected += 1;
                    detected += caught(&s, &f.dimension) as usize;
                }
            }
            // Point the citation at a chunk that was never retrieved.
            let mut s = state.clone();
            s.findings[fi].citations[ci].chunk_id = "not-retrieved-000".into();
            injected += 1;
            detected += caught(&s, &f.dimension) as usize;
        }
    }
    for (li, l) in state.applicable_limits.iter().enumerate() {
        for (ci, c) in l.citations.iter().enumerate() {
            if matches!(c.assertion, Some(Assertion::Limit { .. })) {
                let mut s = state.clone();
                if let Some(Assertion::Limit { deduction, .. }) =
                    &mut s.applicable_limits[li].citations[ci].assertion
                {
                    *deduction = deduction.map(|d| d + 1234.0);
                }
                injected += 1;
                detected += caught(&s, &l.dimension) as usize;
            }
        }
    }
    // Flip the decision to the most permissive outcome when that is not justified.
    if state.decision != Some(Decision::Admissible) {
        let mut s = state.clone();
        s.decision = Some(Decision::Admissible);
        injected += 1;
        detected += caught(&s, "decision") as usize;
    }
    (injected, detected)
}

pub async fn run_suite(engine: &Engine, cases: &[Value]) -> EvalReport {
    let llm = OfflineLlm;
    let mut results = vec![];
    let mut latencies = vec![];
    let mut faults = (0, 0);
    let mut deterministic = true;
    let mut confusion: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();

    for case in cases {
        let t = Instant::now();
        let state = engine
            .analyze_unchecked(case.clone(), &llm, Some(false))
            .await;
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        latencies.push(ms);
        let again = engine
            .analyze_unchecked(case.clone(), &llm, Some(false))
            .await;
        let a = serde_json::to_value(strip_timing(state.to_response())).unwrap_or_default();
        let b = serde_json::to_value(strip_timing(again.to_response())).unwrap_or_default();
        deterministic &= a == b;

        let (fi, fd) = fault_injection(&state, engine);
        faults.0 += fi;
        faults.1 += fd;

        let actual = state.decision.unwrap_or(Decision::NeedsReview);
        let exp = expected(&state.case_id);
        let deduction: f64 = state
            .applicable_limits
            .iter()
            .map(|l| l.deduction_inr.unwrap_or(0.0))
            .fold(0.0, |a, b| a + b);
        let v = state.validation.as_ref();
        let supported = v
            .map(|v| v.counts.get("SUPPORTED").copied().unwrap_or(0))
            .unwrap_or(0);
        let total = v.map(|v| v.verifications.len()).unwrap_or(0);
        let correct = exp.map(|e| e.decision == actual);
        let unsafe_decision =
            exp.is_some_and(|e| e.decision != actual && actual != Decision::NeedsReview);
        let amount_match = exp
            .and_then(|e| e.deduction_inr)
            .filter(|_| {
                matches!(
                    actual,
                    AdmissibleWithLimits | PartiallyAdmissible | Admissible
                )
            })
            .map(|d| (d - deduction).abs() < 0.5);
        if let Some(e) = exp {
            *confusion
                .entry(e.decision.to_string())
                .or_default()
                .entry(actual.to_string())
                .or_default() += 1;
        }
        results.push(CaseResult {
            case_id: state.case_id.clone(),
            expected: exp.map(|e| e.decision),
            actual,
            correct,
            unsafe_decision,
            expected_deduction_inr: exp.and_then(|e| e.deduction_inr),
            actual_deduction_inr: deduction,
            amount_match,
            claims_supported: supported,
            claims_total: total,
            attempts: state.attempt,
            latency_ms: (ms * 100.0).round() / 100.0,
        });
    }

    let labelled: Vec<&CaseResult> = results.iter().filter(|r| r.expected.is_some()).collect();
    let correct = labelled.iter().filter(|r| r.correct == Some(true)).count();
    let required: Vec<&&CaseResult> = labelled
        .iter()
        .filter(|r| r.expected == Some(NeedsReview))
        .collect();
    let mut per_class = BTreeMap::new();
    for class in Decision::ALL {
        let tp = labelled
            .iter()
            .filter(|r| r.expected == Some(class) && r.actual == class)
            .count() as f64;
        let fp = labelled
            .iter()
            .filter(|r| r.expected != Some(class) && r.actual == class)
            .count() as f64;
        let support = labelled
            .iter()
            .filter(|r| r.expected == Some(class))
            .count();
        let precision = if tp + fp > 0.0 { tp / (tp + fp) } else { 0.0 };
        let recall = if support > 0 {
            tp / support as f64
        } else {
            0.0
        };
        let f1 = if precision + recall > 0.0 {
            2.0 * precision * recall / (precision + recall)
        } else {
            0.0
        };
        per_class.insert(
            class.to_string(),
            ClassMetrics {
                precision,
                recall,
                f1,
                support,
            },
        );
    }
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let amount_cases = results.iter().filter(|r| r.amount_match.is_some()).count();
    EvalReport {
        engine_version: crate::ENGINE_VERSION.into(),
        labelled_cases: labelled.len(),
        correct,
        accuracy: if labelled.is_empty() {
            0.0
        } else {
            correct as f64 / labelled.len() as f64
        },
        unsafe_decisions: results.iter().filter(|r| r.unsafe_decision).count(),
        required_abstentions: required.len(),
        abstentions_caught: required.iter().filter(|r| r.actual == NeedsReview).count(),
        amount_cases,
        amount_exact: results
            .iter()
            .filter(|r| r.amount_match == Some(true))
            .count(),
        claims_verified: results.iter().map(|r| r.claims_supported).sum(),
        claims_total: results.iter().map(|r| r.claims_total).sum(),
        faults_injected: faults.0,
        faults_detected: faults.1,
        deterministic,
        latency_p50_ms: percentile(&latencies, 0.5),
        latency_p95_ms: percentile(&latencies, 0.95),
        confusion,
        per_class,
        cases: results,
    }
}

fn strip_timing(mut r: crate::model::AnalysisResponse) -> crate::model::AnalysisResponse {
    for t in &mut r.trace {
        t.elapsed_ms = 0.0;
    }
    r
}
