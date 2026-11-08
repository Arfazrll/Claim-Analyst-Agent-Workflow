//! Case Analysis Agent: input validation, fact normalisation, dimension planning.
//!
//! The input is already structured JSON, so "extraction" means normalisation and
//! dimension detection rather than free-text parsing. Unknown, non-critical fields
//! are tolerated and reported, never fed into any dimension's reasoning.

use std::collections::BTreeMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::dimensions::DIMENSIONS;
use crate::model::{obj, CaseState, HandoffKind, MissingEvidence};
use crate::text_safety::clean;

const KNOWN_TOP_LEVEL: &[&str] = &[
    "case_id",
    "policy_id",
    "policy_start_date",
    "claim_date",
    "sum_insured_inr",
    "continuous_coverage_months",
    "prior_insurer_continuous_years",
    "patient",
    "hospital",
    "treatment",
    "expenses_inr",
    "documents",
    "task",
    "evidence_context",
    "expense_timing",
    "prior_policy",
];

const KNOWN_NESTED: &[(&str, &[&str])] = &[
    ("patient", &["age"]),
    ("hospital", &["name", "network_provider"]),
    (
        "treatment",
        &[
            "type",
            "admission_hours",
            "diagnosis",
            "procedure",
            "pre_existing",
            "experimental",
            "package_charges_agreed",
            "hospital_room_unavailable",
            "patient_cannot_be_moved",
        ],
    ),
];

pub const TREATMENT_TYPES: &[&str] = &["inpatient", "day_care", "domiciliary", "outpatient"];

/// A field-level input problem (maps to HTTP 422).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

fn err(field: &str, message: &str) -> FieldError {
    FieldError {
        field: field.to_string(),
        message: message.to_string(),
    }
}

/// Structural validation of a claim case. Unknown fields are allowed; required
/// fields and wrong types are rejected.
pub fn validate_case(case: &Value) -> Result<(), Vec<FieldError>> {
    let mut errors = vec![];
    let Some(map) = case.as_object() else {
        return Err(vec![err("$", "claim case must be a JSON object")]);
    };
    for key in ["case_id", "policy_start_date", "claim_date"] {
        match map.get(key) {
            Some(Value::String(s)) if !s.trim().is_empty() => {
                if s.len() > 128 {
                    errors.push(err(key, "must be at most 128 characters"));
                }
            }
            Some(_) => errors.push(err(key, "must be a non-empty string")),
            None => errors.push(err(key, "field required")),
        }
    }
    match map.get("treatment") {
        Some(Value::Object(t)) => {
            if let Some(v) = t.get("type") {
                match v.as_str() {
                    Some(s) if TREATMENT_TYPES.contains(&s) => {}
                    _ => errors.push(err(
                        "treatment.type",
                        "must be one of inpatient, day_care, domiciliary, outpatient",
                    )),
                }
            }
            if let Some(v) = t.get("admission_hours") {
                match v.as_f64() {
                    Some(h) if (0.0..=24.0 * 365.0).contains(&h) => {}
                    _ => errors.push(err(
                        "treatment.admission_hours",
                        "must be a number between 0 and 8760",
                    )),
                }
            }
            for flag in ["pre_existing", "experimental", "package_charges_agreed"] {
                if let Some(v) = t.get(flag) {
                    if !v.is_boolean() {
                        errors.push(err(&format!("treatment.{flag}"), "must be a boolean"));
                    }
                }
            }
            for flag in ["hospital_room_unavailable", "patient_cannot_be_moved"] {
                if let Some(v) = t.get(flag) {
                    if !(v.is_boolean() || v.is_null()) {
                        errors.push(err(
                            &format!("treatment.{flag}"),
                            "must be a boolean or null",
                        ));
                    }
                }
            }
            for text in ["diagnosis", "procedure"] {
                if let Some(v) = t.get(text) {
                    if !v.is_string() {
                        errors.push(err(&format!("treatment.{text}"), "must be a string"));
                    }
                }
            }
        }
        Some(_) => errors.push(err("treatment", "must be an object")),
        None => errors.push(err("treatment", "field required")),
    }
    for key in [
        "sum_insured_inr",
        "continuous_coverage_months",
        "prior_insurer_continuous_years",
    ] {
        if let Some(v) = map.get(key) {
            match v.as_f64() {
                Some(n) if n >= 0.0 && n.is_finite() && n <= 1e12 => {}
                _ => errors.push(err(key, "must be a non-negative number")),
            }
        }
    }
    if let Some(v) = map.get("expenses_inr") {
        match v.as_object() {
            Some(exp) => {
                if exp.len() > 64 {
                    errors.push(err("expenses_inr", "too many expense heads (max 64)"));
                }
                for (k, amount) in exp {
                    match amount.as_f64() {
                        Some(n) if n >= 0.0 && n.is_finite() && n <= 1e12 => {}
                        _ => errors.push(err(
                            &format!("expenses_inr.{k}"),
                            "must be a non-negative number",
                        )),
                    }
                }
            }
            None => errors.push(err("expenses_inr", "must be an object of amounts")),
        }
    }
    if let Some(v) = map.get("documents") {
        match v.as_array() {
            Some(a) if a.iter().all(Value::is_string) && a.len() <= 100 => {}
            _ => errors.push(err("documents", "must be a list of strings (max 100)")),
        }
    }
    for key in [
        "patient",
        "hospital",
        "evidence_context",
        "expense_timing",
        "prior_policy",
    ] {
        if let Some(v) = map.get(key) {
            if !(v.is_object() || v.is_null()) {
                errors.push(err(key, "must be an object"));
            }
        }
    }
    if let Some(h) = map.get("hospital").and_then(Value::as_object) {
        if let Some(v) = h.get("network_provider") {
            if !v.is_boolean() {
                errors.push(err("hospital.network_provider", "must be a boolean"));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Normalised facts every dimension reads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Facts {
    pub age: Option<i64>,
    pub sum_insured: f64,
    pub continuous_coverage_months: i64,
    pub prior_insurer_continuous_years: i64,
    pub prior_policy: Map<String, Value>,
    pub days_since_policy_start: i64,
    pub treatment_type: String,
    pub admission_hours: f64,
    pub diagnosis: String,
    pub procedure: String,
    pub pre_existing: bool,
    pub experimental: bool,
    pub package_charges_agreed: bool,
    pub hospital_room_unavailable: Option<bool>,
    pub patient_cannot_be_moved: Option<bool>,
    pub hospital_network_provider: bool,
    pub hospital_name: String,
    pub expenses: BTreeMap<String, f64>,
    pub documents: Vec<String>,
    pub evidence_context: Map<String, Value>,
    pub expense_timing: Option<Map<String, Value>>,
    pub unrecognized_fields: Vec<String>,
    pub explicit_unknowns: Vec<String>,
    /// Filled in just before the unmodelled-risk check runs.
    pub cited_chunk_ids: Vec<String>,
}

impl Facts {
    pub const FIELD_COUNT: usize = 24;

    pub fn expense(&self, head: &str) -> f64 {
        self.expenses.get(head).copied().unwrap_or(0.0)
    }

    pub fn case_text(&self) -> String {
        format!("{} {}", self.diagnosis, self.procedure).to_lowercase()
    }

    pub fn prior_history_received(&self) -> Option<bool> {
        self.prior_policy
            .get("database_and_claim_history_received")
            .and_then(Value::as_bool)
    }
}

/// Parse `YYYY-MM-DD` into days since 1970-01-01 (proleptic Gregorian).
pub fn parse_date_days(s: &str) -> Option<i64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return None;
    }
    let y: i64 = parts[0].parse().ok()?;
    let m: i64 = parts[1].parse().ok()?;
    let d: i64 = parts[2].parse().ok()?;
    if !(1..=12).contains(&m) || d < 1 {
        return None;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let dim = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ][(m - 1) as usize];
    if d > dim {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

fn int_or(v: Option<&Value>, default: i64) -> i64 {
    num(v).map(|f| f as i64).unwrap_or(default)
}

pub fn build_facts(raw: &Value) -> (Facts, Vec<MissingEvidence>) {
    let mut missing = vec![];
    let treatment = obj(raw.get("treatment"));
    let hospital = obj(raw.get("hospital"));
    let patient = obj(raw.get("patient"));

    let start = raw
        .get("policy_start_date")
        .and_then(Value::as_str)
        .and_then(parse_date_days);
    let claim = raw
        .get("claim_date")
        .and_then(Value::as_str)
        .and_then(parse_date_days);
    let days_since_policy_start = match (start, claim) {
        (Some(s), Some(c)) => c - s,
        _ => {
            missing.push(MissingEvidence::new(
                "policy_start_date/claim_date",
                "Missing or unparseable policy start / claim date.",
            ));
            1_000_000
        }
    };
    if raw.get("sum_insured_inr").is_none() {
        missing.push(MissingEvidence::new(
            "sum_insured_inr",
            "Sum insured not supplied; sub-limit amounts cannot be computed.",
        ));
    }

    let expenses: BTreeMap<String, f64> = obj(raw.get("expenses_inr"))
        .into_iter()
        .filter_map(|(k, v)| v.as_f64().map(|n| (k, n)))
        .collect();
    let evidence_context = obj(raw.get("evidence_context"));

    let hospital_name = clean(
        hospital.get("name").and_then(Value::as_str).unwrap_or(""),
        120,
    );
    let mut facts = Facts {
        age: patient.get("age").and_then(Value::as_i64),
        sum_insured: num(raw.get("sum_insured_inr")).unwrap_or(0.0),
        continuous_coverage_months: int_or(raw.get("continuous_coverage_months"), 0),
        prior_insurer_continuous_years: int_or(raw.get("prior_insurer_continuous_years"), 0),
        prior_policy: obj(raw.get("prior_policy")),
        days_since_policy_start,
        treatment_type: treatment
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("inpatient")
            .to_string(),
        admission_hours: num(treatment.get("admission_hours")).unwrap_or(24.0),
        diagnosis: clean(
            treatment
                .get("diagnosis")
                .and_then(Value::as_str)
                .unwrap_or(""),
            200,
        ),
        procedure: clean(
            treatment
                .get("procedure")
                .and_then(Value::as_str)
                .unwrap_or(""),
            200,
        ),
        pre_existing: treatment
            .get("pre_existing")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        experimental: treatment
            .get("experimental")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        package_charges_agreed: treatment
            .get("package_charges_agreed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        hospital_room_unavailable: treatment
            .get("hospital_room_unavailable")
            .and_then(Value::as_bool),
        patient_cannot_be_moved: treatment
            .get("patient_cannot_be_moved")
            .and_then(Value::as_bool),
        hospital_network_provider: hospital
            .get("network_provider")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        hospital_name: if hospital_name.is_empty() {
            "the treating facility".to_string()
        } else {
            hospital_name
        },
        expenses,
        documents: raw
            .get("documents")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|d| d.as_str().map(|s| clean(s, 80)))
                    .collect()
            })
            .unwrap_or_default(),
        evidence_context: evidence_context.clone(),
        expense_timing: raw
            .get("expense_timing")
            .and_then(Value::as_object)
            .cloned(),
        unrecognized_fields: vec![],
        explicit_unknowns: vec![],
        cited_chunk_ids: vec![],
    };

    // Attributes irrelevant to the policy decision: kept, never reasoned over.
    let mut unrecognized: Vec<String> = raw
        .as_object()
        .map(|m| {
            m.keys()
                .filter(|k| !KNOWN_TOP_LEVEL.contains(&k.as_str()))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for (group, known) in KNOWN_NESTED {
        for key in obj(raw.get(*group)).keys() {
            if !known.contains(&key.as_str()) {
                unrecognized.push(format!("{group}.{key}"));
            }
        }
    }
    unrecognized.sort();
    facts.unrecognized_fields = unrecognized;

    // A deliberate `null` in evidence_context means "investigated but unknown":
    // a confident decision cannot assume a value for it.
    for (key, value) in &evidence_context {
        if value.is_null() {
            facts.explicit_unknowns.push(key.clone());
            missing.push(MissingEvidence::new(
                format!("evidence_context.{key}"),
                format!("Case explicitly flags '{key}' as unresolved (null); a confident decision cannot assume a value for this fact."),
            ));
        }
    }
    (facts, missing)
}

pub fn run(state: &mut CaseState) {
    let started = Instant::now();
    let (facts, missing) = build_facts(&state.raw_case);
    state.facts = facts;
    state.missing_fields.extend(missing);

    let applicable: Vec<&str> = DIMENSIONS
        .iter()
        .filter(|d| (d.applies)(&state.facts))
        .map(|d| d.key)
        .collect();
    state.dimensions = applicable.iter().map(|s| s.to_string()).collect();
    state.investigation_checklist = applicable.iter().map(|k| k.replace('_', " ")).collect();

    let mut detail = format!(
        "{} decision dimension(s) apply: {}",
        applicable.len(),
        state.dimensions.join(", ")
    );
    if !state.facts.unrecognized_fields.is_empty() {
        detail.push_str(&format!(
            " | ignored non-policy-relevant fields: {:?}",
            state.facts.unrecognized_fields
        ));
    }
    state.log(
        "CaseAnalysisAgent",
        "extract_facts_and_plan",
        detail,
        None,
        started,
        &["raw_case"],
        &[
            "facts",
            "dimensions",
            "investigation_checklist",
            "missing_fields",
        ],
    );
    let n = state.dimensions.len();
    state.hand_off(
        "CaseAnalysisAgent",
        "PolicyEvidenceAgent",
        format!("{n} policy questions to research"),
        HandoffKind::Forward,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dates_parse() {
        assert_eq!(parse_date_days("1970-01-01"), Some(0));
        assert_eq!(
            parse_date_days("2026-01-20").unwrap() - parse_date_days("2026-01-01").unwrap(),
            19
        );
        assert_eq!(parse_date_days("2024-02-30"), None);
        assert_eq!(parse_date_days("not-a-date"), None);
    }

    #[test]
    fn validation_rejects_missing_required() {
        let errs = validate_case(&json!({"case_id": "X"})).unwrap_err();
        let fields: Vec<_> = errs.iter().map(|e| e.field.as_str()).collect();
        assert!(fields.contains(&"policy_start_date"));
        assert!(fields.contains(&"treatment"));
    }

    #[test]
    fn unknown_fields_tolerated_and_reported() {
        let case = json!({"case_id": "X", "policy_start_date": "2025-01-01", "claim_date": "2025-06-01",
            "treatment": {"type": "inpatient", "favourite_colour": "blue"}, "loyalty_tier": "gold"});
        assert!(validate_case(&case).is_ok());
        let (facts, _) = build_facts(&case);
        assert_eq!(
            facts.unrecognized_fields,
            vec![
                "loyalty_tier".to_string(),
                "treatment.favourite_colour".to_string()
            ]
        );
    }

    #[test]
    fn explicit_null_is_unknown() {
        let case = json!({"case_id": "X", "policy_start_date": "2025-01-01", "claim_date": "2025-06-01",
            "treatment": {}, "evidence_context": {"hospital_registered": null}});
        let (facts, missing) = build_facts(&case);
        assert_eq!(
            facts.explicit_unknowns,
            vec!["hospital_registered".to_string()]
        );
        assert!(missing
            .iter()
            .any(|m| m.field == "evidence_context.hospital_registered"));
    }
}
