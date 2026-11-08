//! Claim Analyst engine: a policy-grounded, five-agent pipeline that decides health
//! insurance claim admissibility with inspectable citations and explicit abstention.
//!
//! ```text
//! Case Analysis -> Policy Evidence -> Coverage & Exclusion -> Decision -> Validation
//!                        ^                                                   |
//!                        +---------- retry (widened retrieval, once) --------+
//! ```
//!
//! Every hand-off is recorded on the state and every agent records what it read,
//! wrote, and a snapshot of the shared state, so the workflow is inspectable.

pub mod agents;
pub mod decision;
pub mod dimensions;
pub mod eval;
pub mod facts;
pub mod interpretation;
pub mod llm;
pub mod model;
pub mod retrieval;
pub mod rules;
pub mod text_safety;
pub mod verification;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

pub use facts::{validate_case, FieldError};
pub use llm::{LlmClient, OfflineLlm};
pub use model::{AnalysisResponse, CaseState, Decision, HandoffKind, ValidationStatus};
pub use retrieval::{HybridRetriever, PolicyChunk, TopK};

pub const ENGINE_VERSION: &str = concat!("claim-engine/", env!("CARGO_PKG_VERSION"), "+rules.11");

/// The bundled policy index (Universal Sompo CSC Individual Health Insurance wording,
/// chunked one clause / definition per chunk).
pub const BUNDLED_POLICY_JSONL: &str = include_str!("../../../data/policy/chunks.jsonl");

#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    pub top_k: TopK,
    pub max_attempts: u32,
    pub interpret_by_default: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            top_k: TopK::default(),
            max_attempts: 2,
            interpret_by_default: false,
        }
    }
}

#[derive(Clone)]
pub struct Engine {
    retriever: Arc<HybridRetriever>,
    corpus: Arc<BTreeMap<String, String>>,
    config: EngineConfig,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("chunks", &self.retriever.len())
            .finish()
    }
}

impl Engine {
    pub fn new(retriever: HybridRetriever, config: EngineConfig) -> Self {
        let corpus = Arc::new(retriever.corpus());
        Engine {
            retriever: Arc::new(retriever),
            corpus,
            config,
        }
    }

    /// Engine over the bundled policy index.
    pub fn bundled() -> Self {
        let retriever = HybridRetriever::from_jsonl(BUNDLED_POLICY_JSONL)
            .expect("bundled policy index is valid");
        Engine::new(retriever, EngineConfig::default())
    }

    pub fn retriever(&self) -> &HybridRetriever {
        &self.retriever
    }

    pub fn config(&self) -> EngineConfig {
        self.config
    }

    /// Validate and analyse one claim case.
    pub async fn analyze(
        &self,
        raw: Value,
        llm: &dyn LlmClient,
        interpret: Option<bool>,
    ) -> Result<CaseState, Vec<FieldError>> {
        validate_case(&raw)?;
        Ok(self.analyze_unchecked(raw, llm, interpret).await)
    }

    /// Analyse a case assumed to be structurally valid.
    pub async fn analyze_unchecked(
        &self,
        raw: Value,
        llm: &dyn LlmClient,
        interpret: Option<bool>,
    ) -> CaseState {
        let interpret = interpret.unwrap_or(self.config.interpret_by_default);
        let case_id = raw
            .get("case_id")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN")
            .to_string();
        let mut state = CaseState::new(case_id, raw);
        facts::run(&mut state);
        loop {
            agents::policy_evidence(&mut state, &self.retriever, self.config.top_k);
            agents::coverage_exclusion(&mut state, llm, interpret, &self.retriever).await;
            decision::run(&mut state, llm).await;
            let last_chance = state.attempt >= self.config.max_attempts;
            agents::validation(&mut state, &self.corpus, last_chance);
            let validation = state
                .validation
                .as_ref()
                .map(|v| v.status)
                .unwrap_or(ValidationStatus::Fail);
            if validation == ValidationStatus::Pass || last_chance {
                let d = state.decision.unwrap_or(Decision::NeedsReview);
                let status = if validation == ValidationStatus::Pass {
                    "PASS"
                } else {
                    "FAIL"
                };
                state.hand_off(
                    "ValidationAgent",
                    "Response",
                    format!("validation {status}; decision {d}"),
                    HandoffKind::Forward,
                );
                return state;
            }
            let v = state.validation.as_ref().expect("validation set");
            let mut failing: Vec<&str> = v
                .verifications
                .iter()
                .filter(|r| r.verdict != model::Verdict::Supported)
                .map(|r| r.dimension.as_str())
                .collect();
            failing.sort();
            failing.dedup();
            let payload = format!(
                "retry {}: {} claim(s) failed verification ({}); widening retrieval",
                state.attempt,
                v.unsupported_claims.len(),
                failing.join(", ")
            );
            state.hand_off(
                "ValidationAgent",
                "PolicyEvidenceAgent",
                payload,
                HandoffKind::Retry,
            );
            state.attempt += 1;
            state.widen_retrieval = true;
            state.reset_analysis();
        }
    }
}
