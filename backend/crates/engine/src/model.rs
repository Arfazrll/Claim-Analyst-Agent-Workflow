//! Typed state shared by every agent, plus the public decision contract.
//!
//! Agents exchange one [`CaseState`] instead of free-form text. Each agent owns a
//! distinct part of it (facts, evidence, findings, decision, validation), so every
//! contribution can be inspected on its own. Every material citation carries a
//! machine-checkable [`Assertion`] that the Validation Agent re-checks against the
//! retrieved chunk text.

use std::collections::BTreeMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::facts::Facts;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Decision {
    Admissible,
    AdmissibleWithLimits,
    PartiallyAdmissible,
    NotAdmissible,
    NeedsReview,
}

impl Decision {
    pub const ALL: [Decision; 5] = [
        Decision::Admissible,
        Decision::AdmissibleWithLimits,
        Decision::PartiallyAdmissible,
        Decision::NotAdmissible,
        Decision::NeedsReview,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Decision::Admissible => "ADMISSIBLE",
            Decision::AdmissibleWithLimits => "ADMISSIBLE_WITH_LIMITS",
            Decision::PartiallyAdmissible => "PARTIALLY_ADMISSIBLE",
            Decision::NotAdmissible => "NOT_ADMISSIBLE",
            Decision::NeedsReview => "NEEDS_REVIEW",
        }
    }

    pub fn parse(s: &str) -> Option<Decision> {
        Decision::ALL
            .into_iter()
            .find(|d| d.as_str().eq_ignore_ascii_case(s.trim()))
    }

    /// Rank by how much the decision pays out; used to make sure optional steps
    /// (LLM interpretation) never move a case to a more permissive outcome.
    pub fn permissiveness(&self) -> u8 {
        match self {
            Decision::NotAdmissible => 0,
            Decision::NeedsReview => 1,
            Decision::PartiallyAdmissible => 2,
            Decision::AdmissibleWithLimits => 3,
            Decision::Admissible => 4,
        }
    }
}

impl std::fmt::Display for Decision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FindingStatus {
    SupportsAdmissible,
    SupportsLimit,
    SupportsExclusion,
    InsufficientEvidence,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingSource {
    Rules,
    Llm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueUnit {
    pub value: i64,
    pub unit: String,
}

/// What a citation claims its chunk says, in a form the verifier can check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Assertion {
    /// Numbers with a unit (days / months / years) that must appear in the chunk.
    Threshold {
        values: Vec<ValueUnit>,
        #[serde(default)]
        must_contain: Vec<String>,
    },
    /// Literal phrases that must all appear (and at least one of each `any_of` group).
    Phrases {
        #[serde(default)]
        must_contain: Vec<String>,
        #[serde(default)]
        any_of: Vec<Vec<String>>,
    },
    /// `<pct>% of Sum Insured [or Rs <flat>]` bound to a keyword clause, plus the
    /// cap / deduction arithmetic derived from it.
    Limit {
        keyword: String,
        percent: Option<f64>,
        flat: Option<f64>,
        sum_insured: Option<f64>,
        multiplier: f64,
        claimed: Option<f64>,
        cap: Option<f64>,
        deduction: Option<f64>,
    },
    /// A term that may appear only in the allowed chunks across the whole policy
    /// (backs claims of the form "the policy has no explicit clause on X").
    Absence {
        term: String,
        #[serde(default)]
        allowed_chunk_ids: Vec<String>,
    },
    /// A quote that must appear verbatim (case / whitespace-insensitive).
    Quote { quote: String },
}

impl Assertion {
    pub fn threshold(pairs: &[(i64, &str)], must_contain: &[&str]) -> Self {
        Assertion::Threshold {
            values: pairs
                .iter()
                .map(|(v, u)| ValueUnit {
                    value: *v,
                    unit: (*u).to_string(),
                })
                .collect(),
            must_contain: must_contain.iter().map(|s| s.to_string()).collect(),
        }
    }

    pub fn phrases(must: &[&str]) -> Self {
        Assertion::Phrases {
            must_contain: must.iter().map(|s| s.to_string()).collect(),
            any_of: vec![],
        }
    }

    pub fn phrases_owned(must: Vec<String>) -> Self {
        Assertion::Phrases {
            must_contain: must,
            any_of: vec![],
        }
    }

    pub fn keyword(&self) -> Option<&str> {
        match self {
            Assertion::Limit { keyword, .. } => Some(keyword.as_str()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Citation {
    pub claim: String,
    pub source: String,
    pub page: i64,
    pub section: String,
    pub chunk_id: String,
    pub rerank_score: Option<f64>,
    pub assertion: Option<Assertion>,
}

/// One retrieved policy chunk with its per-stage retrieval provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub chunk_id: String,
    pub section: String,
    pub label: String,
    pub page_start: i64,
    pub page_end: i64,
    pub text: String,
    pub dense_rank: Option<usize>,
    pub dense_score: Option<f64>,
    pub sparse_rank: Option<usize>,
    pub sparse_score: Option<f64>,
    pub fused_score: Option<f64>,
    pub rerank_score: Option<f64>,
}

impl EvidenceItem {
    pub fn cite(&self, claim: impl Into<String>, assertion: Option<Assertion>) -> Citation {
        Citation {
            claim: claim.into(),
            source: "policy.pdf".to_string(),
            page: self.page_start,
            section: self.section.clone(),
            chunk_id: self.chunk_id.clone(),
            rerank_score: self.rerank_score,
            assertion,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub dimension: String,
    pub applicable: bool,
    pub statement: String,
    pub status: FindingStatus,
    pub citations: Vec<Citation>,
    pub confidence: f64,
    pub assumptions: Vec<String>,
    pub source: FindingSource,
}

impl Finding {
    pub fn new(
        dimension: &str,
        status: FindingStatus,
        statement: impl Into<String>,
        confidence: f64,
    ) -> Self {
        Finding {
            dimension: dimension.to_string(),
            applicable: true,
            statement: statement.into(),
            status,
            citations: vec![],
            confidence,
            assumptions: vec![],
            source: FindingSource::Rules,
        }
    }

    pub fn cite(mut self, c: Citation) -> Self {
        self.citations.push(c);
        self
    }

    pub fn assume(mut self, a: impl IntoIterator<Item = String>) -> Self {
        self.assumptions.extend(a);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplicableLimit {
    pub description: String,
    pub dimension: String,
    pub deduction_inr: Option<f64>,
    pub citations: Vec<Citation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingEvidence {
    pub field: String,
    pub reason: String,
}

impl MissingEvidence {
    pub fn new(field: impl Into<String>, reason: impl Into<String>) -> Self {
        MissingEvidence {
            field: field.into(),
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Supported,
    Unsupported,
    Contradicted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

impl CheckResult {
    pub fn new(name: impl Into<String>, passed: bool, detail: impl Into<String>) -> Self {
        CheckResult {
            name: name.into(),
            passed,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimKind {
    PolicyClaim,
    LimitClaim,
    DecisionClaim,
}

/// One row of the decision-claim -> citation -> chunk -> verdict audit trail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaimVerification {
    pub claim_id: String,
    pub kind: ClaimKind,
    pub dimension: String,
    pub claim: String,
    pub chunk_id: Option<String>,
    pub page: Option<i64>,
    pub section: Option<String>,
    pub verdict: Verdict,
    pub reason: String,
    pub checks: Vec<CheckResult>,
    pub excerpt: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ValidationStatus {
    Pass,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationOutcome {
    pub status: ValidationStatus,
    pub unsupported_claims: Vec<String>,
    pub notes: String,
    pub verifications: Vec<ClaimVerification>,
    pub counts: BTreeMap<String, usize>,
    pub attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub facts: usize,
    pub dimensions: usize,
    pub evidence_chunks: usize,
    pub findings: usize,
    pub limits: usize,
    pub missing: usize,
    pub decision: Option<Decision>,
    pub validation: Option<ValidationStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    pub agent: String,
    pub action: String,
    pub detail: String,
    pub elapsed_ms: f64,
    pub retrieval_count: Option<usize>,
    pub attempt: u32,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub snapshot: StateSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HandoffKind {
    Forward,
    Retry,
}

/// A visible state transition between two agents (including retries).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Handoff {
    pub from_agent: String,
    pub to_agent: String,
    pub payload: String,
    pub kind: HandoffKind,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmInterpretationReport {
    pub enabled: bool,
    #[serde(default)]
    pub called: bool,
    #[serde(default)]
    pub reply_received: bool,
    #[serde(default)]
    pub reused_first_reply: bool,
    #[serde(default)]
    pub accepted: Vec<Value>,
    #[serde(default)]
    pub rejected: Vec<Value>,
    pub error: Option<String>,
}

/// Structured state threaded through the agent pipeline.
#[derive(Debug, Clone)]
pub struct CaseState {
    pub case_id: String,
    pub raw_case: Value,
    pub facts: Facts,
    pub dimensions: Vec<String>,
    pub investigation_checklist: Vec<String>,
    pub missing_fields: Vec<MissingEvidence>,
    pub evidence_by_dimension: BTreeMap<String, Vec<EvidenceItem>>,
    pub findings: Vec<Finding>,
    pub applicable_limits: Vec<ApplicableLimit>,
    pub decision: Option<Decision>,
    pub confidence: f64,
    pub rationale: String,
    pub rationale_source: String,
    pub llm_interpretation: LlmInterpretationReport,
    /// First model reply, reused on a validation retry so a case costs one model call.
    pub llm_cache: Option<Option<String>>,
    /// Clauses offered to the model (verifiable, not counted as retrieval).
    pub llm_evidence: BTreeMap<String, EvidenceItem>,
    pub validation: Option<ValidationOutcome>,
    pub trace: Vec<TraceEvent>,
    pub handoffs: Vec<Handoff>,
    pub attempt: u32,
    pub widen_retrieval: bool,
}

impl CaseState {
    pub fn new(case_id: String, raw_case: Value) -> Self {
        CaseState {
            case_id,
            raw_case,
            facts: Facts::default(),
            dimensions: vec![],
            investigation_checklist: vec![],
            missing_fields: vec![],
            evidence_by_dimension: BTreeMap::new(),
            findings: vec![],
            applicable_limits: vec![],
            decision: None,
            confidence: 0.0,
            rationale: String::new(),
            rationale_source: "template".to_string(),
            llm_interpretation: LlmInterpretationReport::default(),
            llm_cache: None,
            llm_evidence: BTreeMap::new(),
            validation: None,
            trace: vec![],
            handoffs: vec![],
            attempt: 1,
            widen_retrieval: false,
        }
    }

    pub fn snapshot(&self) -> StateSnapshot {
        StateSnapshot {
            facts: Facts::FIELD_COUNT,
            dimensions: self.dimensions.len(),
            evidence_chunks: self.evidence_by_dimension.values().map(Vec::len).sum(),
            findings: self.findings.len(),
            limits: self.applicable_limits.len(),
            missing: self.missing_fields.len(),
            decision: self.decision,
            validation: self.validation.as_ref().map(|v| v.status),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn log(
        &mut self,
        agent: &str,
        action: &str,
        detail: String,
        retrieval_count: Option<usize>,
        started: Instant,
        reads: &[&str],
        writes: &[&str],
    ) {
        let elapsed_ms = (started.elapsed().as_secs_f64() * 1000.0 * 100.0).round() / 100.0;
        let snapshot = self.snapshot();
        self.trace.push(TraceEvent {
            agent: agent.to_string(),
            action: action.to_string(),
            detail,
            elapsed_ms,
            retrieval_count,
            attempt: self.attempt,
            reads: reads.iter().map(|s| s.to_string()).collect(),
            writes: writes.iter().map(|s| s.to_string()).collect(),
            snapshot,
        });
    }

    pub fn hand_off(&mut self, from: &str, to: &str, payload: String, kind: HandoffKind) {
        self.handoffs.push(Handoff {
            from_agent: from.into(),
            to_agent: to.into(),
            payload,
            kind,
        });
    }

    /// Clear everything derived from evidence so a retry recomputes it cleanly.
    pub fn reset_analysis(&mut self) {
        self.evidence_by_dimension.clear();
        self.findings.clear();
        self.applicable_limits.clear();
        self.missing_fields.retain(|m| {
            m.field.starts_with("evidence_context.")
                || m.field == "policy_start_date/claim_date"
                || m.field == "sum_insured_inr"
        });
        self.decision = None;
        self.confidence = 0.0;
        self.rationale.clear();
        self.validation = None;
        self.llm_interpretation = LlmInterpretationReport::default();
    }

    pub fn all_citations(&self) -> Vec<Citation> {
        let mut seen: Vec<((String, String), Citation)> = vec![];
        let mut push = |c: &Citation| {
            let key = (c.chunk_id.clone(), c.claim.clone());
            if let Some(slot) = seen.iter_mut().find(|(k, _)| *k == key) {
                slot.1 = c.clone();
            } else {
                seen.push((key, c.clone()));
            }
        };
        for f in &self.findings {
            f.citations.iter().for_each(&mut push);
        }
        for l in &self.applicable_limits {
            l.citations.iter().for_each(&mut push);
        }
        seen.into_iter().map(|(_, c)| c).collect()
    }

    pub fn lookup_chunk(&self, chunk_id: &str) -> Option<&EvidenceItem> {
        self.evidence_by_dimension
            .values()
            .flat_map(|v| v.iter())
            .chain(self.llm_evidence.values())
            .find(|e| e.chunk_id == chunk_id)
    }

    fn excerpt_for(&self, chunk_id: &str, limit: usize) -> Option<String> {
        self.lookup_chunk(chunk_id)
            .map(|e| crate::text_safety::truncate_words(&e.text, limit))
    }

    pub fn amounts(&self, decision: Decision) -> Amounts {
        let expense_breakdown_inr = self.facts.expenses.clone();
        let claimed: f64 = expense_breakdown_inr.values().fold(0.0, |a, b| a + b);
        let deductions: f64 = self
            .applicable_limits
            .iter()
            .map(|l| l.deduction_inr.unwrap_or(0.0))
            .fold(0.0, |a, b| a + b);
        let payable = match decision {
            Decision::NeedsReview => None,
            Decision::NotAdmissible => Some(0.0),
            _ => Some((claimed - deductions).max(0.0)),
        };
        Amounts {
            claimed_total_inr: claimed,
            total_deductions_inr: deductions,
            estimated_payable_inr: payable,
            expense_breakdown_inr,
        }
    }

    /// Project the final state onto the public decision contract.
    pub fn to_response(&self) -> AnalysisResponse {
        let decision = self.decision.unwrap_or(Decision::NeedsReview);
        AnalysisResponse {
            case_id: self.case_id.clone(),
            decision,
            confidence: (self.confidence * 100.0).round() / 100.0,
            key_findings: self
                .findings
                .iter()
                .filter(|f| f.applicable)
                .map(|f| f.statement.clone())
                .collect(),
            findings: self
                .findings
                .iter()
                .map(|f| FindingOut {
                    dimension: f.dimension.clone(),
                    status: f.status,
                    statement: f.statement.clone(),
                    confidence: f.confidence,
                    applicable: f.applicable,
                    chunk_ids: f.citations.iter().map(|c| c.chunk_id.clone()).collect(),
                    assumptions: f.assumptions.clone(),
                    source: f.source,
                })
                .collect(),
            assumptions: self
                .findings
                .iter()
                .flat_map(|f| {
                    f.assumptions.iter().map(|a| AssumptionOut {
                        dimension: f.dimension.clone(),
                        assumption: a.clone(),
                    })
                })
                .collect(),
            unmodelled_policy_risks: self
                .findings
                .iter()
                .filter(|f| f.dimension == "unmodelled_policy_risk" && f.applicable)
                .map(|f| RiskOut {
                    statement: f.statement.clone(),
                    chunk_ids: f.citations.iter().map(|c| c.chunk_id.clone()).collect(),
                })
                .collect(),
            amounts: self.amounts(decision),
            applicable_limits: self
                .applicable_limits
                .iter()
                .map(|l| LimitOut {
                    description: l.description.clone(),
                    deduction_inr: l.deduction_inr,
                    dimension: l.dimension.clone(),
                })
                .collect(),
            missing_evidence: self.missing_fields.clone(),
            citations: self
                .all_citations()
                .into_iter()
                .map(|c| {
                    let excerpt = self.excerpt_for(&c.chunk_id, 600);
                    CitationOut {
                        citation: c,
                        excerpt,
                    }
                })
                .collect(),
            validation: self.validation.clone().unwrap_or(ValidationOutcome {
                status: ValidationStatus::Pass,
                unsupported_claims: vec![],
                notes: String::new(),
                verifications: vec![],
                counts: BTreeMap::new(),
                attempts: self.attempt,
            }),
            rationale: self.rationale.clone(),
            rationale_source: self.rationale_source.clone(),
            llm_interpretation: self.llm_interpretation.clone(),
            trace: self.trace.clone(),
            handoffs: self.handoffs.clone(),
            unrecognized_fields: self.facts.unrecognized_fields.clone(),
            engine_version: crate::ENGINE_VERSION.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Amounts {
    pub claimed_total_inr: f64,
    pub total_deductions_inr: f64,
    pub estimated_payable_inr: Option<f64>,
    pub expense_breakdown_inr: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindingOut {
    pub dimension: String,
    pub status: FindingStatus,
    pub statement: String,
    pub confidence: f64,
    pub applicable: bool,
    pub chunk_ids: Vec<String>,
    pub assumptions: Vec<String>,
    pub source: FindingSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssumptionOut {
    pub dimension: String,
    pub assumption: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskOut {
    pub statement: String,
    pub chunk_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitOut {
    pub description: String,
    pub deduction_inr: Option<f64>,
    pub dimension: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CitationOut {
    #[serde(flatten)]
    pub citation: Citation,
    pub excerpt: Option<String>,
}

/// The public decision contract returned by the API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisResponse {
    pub case_id: String,
    pub decision: Decision,
    pub confidence: f64,
    pub key_findings: Vec<String>,
    pub findings: Vec<FindingOut>,
    pub assumptions: Vec<AssumptionOut>,
    pub unmodelled_policy_risks: Vec<RiskOut>,
    pub amounts: Amounts,
    pub applicable_limits: Vec<LimitOut>,
    pub missing_evidence: Vec<MissingEvidence>,
    pub citations: Vec<CitationOut>,
    pub validation: ValidationOutcome,
    pub rationale: String,
    pub rationale_source: String,
    pub llm_interpretation: LlmInterpretationReport,
    pub trace: Vec<TraceEvent>,
    pub handoffs: Vec<Handoff>,
    pub unrecognized_fields: Vec<String>,
    pub engine_version: String,
}

/// Helper: object view of a JSON value (empty map for non-objects).
pub fn obj(v: Option<&Value>) -> Map<String, Value> {
    v.and_then(Value::as_object).cloned().unwrap_or_default()
}
