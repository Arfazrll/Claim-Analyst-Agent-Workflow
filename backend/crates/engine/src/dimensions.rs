//! The decision-dimension registry: the reasoning core.
//!
//! Each dimension is an (applicability check, retrieval query, evaluator) triple. An
//! evaluator only asserts a conclusion when the number or condition it needs was
//! found in the text of a chunk retrieved for that dimension. If the number cannot be
//! parsed out of the retrieved evidence, or a required case fact is missing, it
//! reports INSUFFICIENT_EVIDENCE rather than guessing, which lets the Decision Agent
//! abstain safely.
//!
//! Evidence selection is assertion-aware: instead of blindly taking the top-ranked
//! chunk, an evaluator takes the highest-ranked retrieved chunk that actually states
//! what it needs (falling back to the top chunk, which verification then flags).

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::facts::Facts;
use crate::model::{
    ApplicableLimit, Assertion, EvidenceItem, Finding, FindingStatus as S, MissingEvidence,
};
use crate::rules as rx;
use crate::text_safety::{fmt_inr, fmt_pct, has_term, norm};

pub type EvalResult = (Finding, Vec<ApplicableLimit>, Vec<MissingEvidence>);

pub struct Dimension {
    pub key: &'static str,
    pub title: &'static str,
    pub applies: fn(&Facts) -> bool,
    pub query: fn(&Facts) -> String,
    pub evaluate: fn(&Facts, &[EvidenceItem]) -> EvalResult,
}

pub const UNMODELLED: &str = "unmodelled_policy_risk";

pub static DIMENSIONS: [Dimension; 11] = [
    Dimension {
        key: "initial_waiting_period",
        title: "Initial waiting period",
        applies: |_| true,
        query: |_| {
            "30 days waiting period all claims exceptions continuous coverage without break".into()
        },
        evaluate: initial_wait,
    },
    Dimension {
        key: "named_disease_first_year_waiting_period",
        title: "First-year named-disease wait",
        applies: named_disease_applies,
        query: |_| {
            "first year of operation waiting period named diseases cataract hernia hysterectomy waiver".into()
        },
        evaluate: named_disease,
    },
    Dimension {
        key: "pre_existing_disease_waiting_period",
        title: "Pre-existing disease wait",
        applies: |f| f.pre_existing,
        query: |_| {
            "pre-existing diseases waiting period months continuous coverage portability reduction"
                .into()
        },
        evaluate: pre_existing,
    },
    Dimension {
        key: "hospital_definition",
        title: "Hospital definition",
        applies: |f| f.treatment_type != "domiciliary",
        query: |_| {
            "definition of Hospital registration minimum criteria in-patient beds qualified nursing"
                .into()
        },
        evaluate: hospital_def,
    },
    Dimension {
        key: "domiciliary_treatment_conditions",
        title: "Domiciliary conditions",
        applies: |f| f.treatment_type == "domiciliary",
        query: |_| {
            "domiciliary treatment condition patient cannot be removed non-availability of room in hospital".into()
        },
        evaluate: domiciliary,
    },
    Dimension {
        key: "day_care_less_than_24h",
        title: "Day care / under 24h",
        applies: daycare_applies,
        query: |_| {
            "day care treatment less than 24 hours anesthesia technological advancement list of procedures".into()
        },
        evaluate: daycare,
    },
    Dimension {
        key: "pre_post_hospitalization_window",
        title: "Pre/post hospitalisation window",
        applies: |f| {
            f.expense("pre_hospitalization") > 0.0 || f.expense("post_hospitalization") > 0.0
        },
        query: |_| {
            "pre-hospitalisation post-hospitalisation days immediately preceding following expenses"
                .into()
        },
        evaluate: pre_post,
    },
    Dimension {
        key: "cosmetic_exclusion",
        title: "Cosmetic exclusion",
        applies: cosmetic_applies,
        query: |_| "cosmetic aesthetic treatment plastic surgery exclusion injury disease".into(),
        evaluate: cosmetic,
    },
    Dimension {
        key: "experimental_unproven_treatment",
        title: "Experimental treatment",
        applies: |f| f.experimental,
        query: |_| {
            "unproven experimental treatment definition established medical practice not approved"
                .into()
        },
        evaluate: experimental,
    },
    Dimension {
        key: "category_sub_limits",
        title: "Category sub-limits",
        applies: |f| {
            matches!(
                f.treatment_type.as_str(),
                "inpatient" | "day_care" | "domiciliary"
            )
        },
        query: |_| {
            "room boarding nursing sub limit surgeon fees anesthesia medicines package sum insured domiciliary ambulance".into()
        },
        evaluate: sub_limits,
    },
    Dimension {
        key: UNMODELLED,
        title: "Unmodelled policy risk",
        applies: |_| true,
        query: |f| {
            format!("{} {} exclusion not covered", f.diagnosis, f.procedure)
                .trim()
                .to_string()
        },
        evaluate: unmodelled,
    },
];

pub fn by_key(key: &str) -> Option<&'static Dimension> {
    DIMENSIONS.iter().find(|d| d.key == key)
}

// ---------------------------------------------------------------- helpers

fn no_evidence(dim: &str, why: &str) -> EvalResult {
    let finding = Finding::new(
        dim,
        S::InsufficientEvidence,
        format!("{why} Treat as insufficient evidence for this dimension."),
        0.2,
    );
    (finding, vec![], vec![MissingEvidence::new(dim, why)])
}

/// Highest-ranked chunk satisfying `pred`, else the top chunk.
fn pick(evidence: &[EvidenceItem], pred: impl Fn(&str) -> bool) -> &EvidenceItem {
    evidence
        .iter()
        .find(|e| pred(&e.text))
        .unwrap_or(&evidence[0])
}

fn contains(text: &str, phrase: &str) -> bool {
    norm(text).contains(&norm(phrase))
}

fn find_source<'a>(evidence: &'a [EvidenceItem], keyword: &str) -> &'a EvidenceItem {
    pick(evidence, |t| {
        t.to_lowercase().contains(&keyword.to_lowercase())
    })
}

// ---------------------------------------------------------------- 1. initial wait

fn initial_wait(f: &Facts, evidence: &[EvidenceItem]) -> EvalResult {
    let dim = "initial_waiting_period";
    if evidence.is_empty() {
        return no_evidence(
            dim,
            "Could not retrieve the policy's initial waiting-period clause.",
        );
    }
    let top = pick(evidence, |t| rx::extract_waiting_days(t).is_some());
    let Some(days) = rx::extract_waiting_days(&top.text) else {
        return no_evidence(
            dim,
            "Retrieved clause did not contain a parseable waiting-period day count.",
        );
    };
    let since = f.days_since_policy_start;
    let continuity_ok = f.continuous_coverage_months > 0 || f.prior_insurer_continuous_years >= 1;
    let mut assumptions = vec![];
    if continuity_ok && f.continuous_coverage_months > 0 {
        assumptions.push(
            "Continuous-coverage months reported in the case are assumed to be break-free."
                .to_string(),
        );
    }
    if continuity_ok && f.continuous_coverage_months == 0 {
        assumptions.push("The prior-insurer route also requires proof the insured was unaware of the illness; this is not verifiable from the case data.".to_string());
    }
    let assertion = Assertion::threshold(&[(days, "days")], &[]);
    let finding = if since >= days || continuity_ok {
        let reason = if continuity_ok {
            format!("policy has been continuously in force ({} months) or prior Indian-insurer continuity applies", f.continuous_coverage_months)
        } else {
            format!("claim date is {since} days after policy inception")
        };
        Finding::new(
            dim,
            S::SupportsAdmissible,
            format!("Initial {days}-day waiting period is satisfied: {reason}."),
            0.9,
        )
        .cite(top.cite(
            format!("{days}-day initial waiting period and continuity exceptions"),
            Some(assertion),
        ))
        .assume(if since < days { assumptions } else { vec![] })
    } else {
        Finding::new(
            dim,
            S::SupportsExclusion,
            format!("Claim date is only {since} days after policy inception, inside the policy's {days}-day initial waiting period, and no continuous-coverage exception applies."),
            0.9,
        )
        .cite(top.cite(format!("{days}-day initial waiting period"), Some(assertion)))
    };
    (finding, vec![], vec![])
}

// ---------------------------------------------------------------- 2. named diseases

const NAMED_DISEASE_KEYWORDS: &[&str] = &[
    "cataract",
    "benign prostatic",
    "myomectomy",
    "hysterectomy",
    "hernia",
    "hydrocele",
    "fistula",
    "piles",
    "arthritis",
    "gout",
    "rheumatism",
    "joint replacement",
    "sinusitis",
    "stone in",
    "urinary",
    "biliary",
    "dilatation and curettage",
    "d&c",
    "cyst",
    "polyp",
    "tumour",
    "tumor",
    "nodule",
    "dialysis",
    "tonsil",
    "gastric ulcer",
    "duodenal ulcer",
];

/// Clinical / lay synonyms for items on the first-year list (case wording prefix ->
/// the policy's own wording). A synonym match is weaker evidence than the policy's
/// literal term, so inside the waiting period it abstains (NEEDS_REVIEW) instead of
/// rejecting: it can never approve or reject on its own.
pub const CLINICAL_SYNONYMS: &[(&str, &str)] = &[
    ("phacoemulsification", "Cataract"),
    ("phaco", "Cataract"),
    ("intraocular lens", "Cataract"),
    ("intraocular insert", "Cataract"),
    ("clouding of the crystalline", "Cataract"),
    ("lens opacit", "Cataract"),
    ("gallstone", "Stone in the urinary and biliary systems"),
    ("gall stone", "Stone in the urinary and biliary systems"),
    ("cholelithiasis", "Stone in the urinary and biliary systems"),
    (
        "choledocholithiasis",
        "Stone in the urinary and biliary systems",
    ),
    ("kidney stone", "Stone in the urinary and biliary systems"),
    ("renal calcul", "Stone in the urinary and biliary systems"),
    (
        "ureteric calcul",
        "Stone in the urinary and biliary systems",
    ),
    (
        "nephrolithiasis",
        "Stone in the urinary and biliary systems",
    ),
    ("urolithiasis", "Stone in the urinary and biliary systems"),
    ("haemorrhoid", "Piles"),
    ("hemorrhoid", "Piles"),
    ("enlarged prostate", "Benign Prostatic Hypertrophy"),
    ("prostatic enlargement", "Benign Prostatic Hypertrophy"),
    ("bph", "Benign Prostatic Hypertrophy"),
    ("fibroid", "Myomectomy, Hysterectomy"),
    ("leiomyoma", "Myomectomy, Hysterectomy"),
    ("osteoarthritis", "Arthritis"),
    ("knee replacement", "Joint replacement"),
    ("hip replacement", "Joint replacement"),
    ("arthroplasty", "Joint replacement"),
    ("peptic ulcer", "Gastric and duodenal ulcers"),
    ("rhinosinusitis", "Sinusitis"),
    ("adenoid", "adenoids"),
];

fn direct_named_match(f: &Facts) -> bool {
    let text = f.case_text();
    NAMED_DISEASE_KEYWORDS.iter().any(|k| has_term(&text, k))
}

/// (case wording, policy wording) pairs matched through the synonym lexicon.
pub fn synonym_matches(f: &Facts) -> Vec<(&'static str, &'static str)> {
    let text = f.case_text();
    let mut out: Vec<(&str, &str)> = CLINICAL_SYNONYMS
        .iter()
        .filter(|(s, _)| has_term(&text, s))
        .copied()
        .collect();
    out.dedup_by(|a, b| a.1 == b.1);
    out
}

fn named_disease_applies(f: &Facts) -> bool {
    matches!(f.treatment_type.as_str(), "inpatient" | "day_care")
        && (direct_named_match(f) || !synonym_matches(f).is_empty())
}

fn named_disease(f: &Facts, evidence: &[EvidenceItem]) -> EvalResult {
    let dim = "named_disease_first_year_waiting_period";
    if evidence.is_empty() {
        return no_evidence(
            dim,
            "Could not retrieve the first-year named-disease waiting-period clause.",
        );
    }
    let direct = direct_named_match(f);
    let synonyms = synonym_matches(f);
    let top = pick(evidence, |t| {
        contains(t, "first year") && rx::extract_years(t).is_some()
    });
    let Some(years) = rx::extract_years(&top.text) else {
        return no_evidence(
            dim,
            "Retrieved clause did not contain a parseable first-year wait duration.",
        );
    };
    let threshold_days = years * 365;
    let received = f.prior_history_received();
    let waived = f.prior_insurer_continuous_years >= 1 && received.unwrap_or(true);
    let since = f.days_since_policy_start;
    let mut assumptions = vec![];
    if waived && received.is_none() {
        assumptions.push(
            "Prior-insurer claim history is assumed received (the case does not say).".to_string(),
        );
    }
    let assertion = Assertion::threshold(&[(years, "year")], &["first year"]);
    let finding = if since >= threshold_days {
        Finding::new(dim, S::SupportsAdmissible, format!("Policy has been in force for {since} days, past the {years}-year named-disease waiting period."), 0.85)
            .cite(top.cite(format!("{years}-year named-disease waiting period elapsed"), Some(assertion)))
    } else if waived {
        Finding::new(
            dim,
            S::SupportsAdmissible,
            format!(
                "Named-disease waiting period would otherwise apply (policy only {since} days old), but is waived because of {} completed continuous year(s) under a prior Indian individual health insurer with claim history received.",
                f.prior_insurer_continuous_years
            ),
            0.8,
        )
        .cite(top.cite("waiting period waived for continuous prior Indian-insurer coverage", Some(assertion)))
        .assume(assumptions)
    } else if !direct {
        // Only a synonym links the case to the list: hold for a reviewer rather than reject.
        let (case_word, policy_word) = synonyms[0];
        let listed = pick(evidence, |t| contains(t, policy_word));
        let statement = format!(
            "The case wording '{case_word}' appears to describe '{policy_word}', which is on the policy's first-year named-disease list, and the policy is only {since} days old with no prior-insurer waiver. The match comes from a clinical synonym, not the policy's own term, so a reviewer should confirm before the {years}-year wait is applied."
        );
        let finding = Finding::new(dim, S::InsufficientEvidence, statement.clone(), 0.35)
            .cite(listed.cite(
                format!("first-year list names '{policy_word}'"),
                Some(Assertion::phrases_owned(vec![
                    policy_word.to_string(),
                    "first year".into(),
                ])),
            ))
            .assume([format!(
                "Clinical synonym lexicon: '{case_word}' -> '{policy_word}'."
            )]);
        return (
            finding,
            vec![],
            vec![MissingEvidence::new(
                "named_disease_synonym_confirmation",
                statement,
            )],
        );
    } else {
        Finding::new(
            dim,
            S::SupportsExclusion,
            format!("Diagnosis/procedure falls on the policy's first-year named-disease list, the policy is only {since} days old, and no prior-insurer continuity waiver applies."),
            0.85,
        )
        .cite(top.cite(format!("{years}-year named-disease waiting period"), Some(assertion)))
    };
    (finding, vec![], vec![])
}

// ---------------------------------------------------------------- 3. pre-existing

fn pre_existing(f: &Facts, evidence: &[EvidenceItem]) -> EvalResult {
    let dim = "pre_existing_disease_waiting_period";
    if evidence.is_empty() {
        return no_evidence(
            dim,
            "Could not retrieve the pre-existing-disease waiting-period clause.",
        );
    }
    let top = pick(evidence, |t| rx::extract_pre_existing_months(t).is_some());
    let Some(needed) = rx::extract_pre_existing_months(&top.text) else {
        return no_evidence(
            dim,
            "Retrieved clause did not contain a parseable month count.",
        );
    };
    let received = f.prior_history_received();
    let portability = if received.unwrap_or(f.prior_insurer_continuous_years > 0) {
        f.prior_insurer_continuous_years * 12
    } else {
        0
    };
    let effective = f.continuous_coverage_months + portability;
    let mut assumptions = vec![];
    if portability > 0 && received.is_none() {
        assumptions.push(
            "Prior-insurer claim history is assumed received (the case does not say).".to_string(),
        );
    }
    let assertion = Assertion::threshold(&[(needed, "months")], &[]);
    let finding = if effective >= needed {
        Finding::new(
            dim,
            S::SupportsAdmissible,
            format!("Effective continuous coverage ({effective} months, including any portability credit) meets the {needed}-month pre-existing-disease waiting period."),
            0.85,
        )
        .cite(top.cite(format!("{needed}-month pre-existing disease waiting period satisfied"), Some(assertion)))
        .assume(assumptions)
    } else {
        Finding::new(
            dim,
            S::SupportsExclusion,
            format!("Effective continuous coverage is {effective} months, short of the {needed}-month pre-existing-disease waiting period ({} months remaining).", needed - effective),
            0.85,
        )
        .cite(top.cite(format!("{needed}-month pre-existing disease waiting period not met"), Some(assertion)))
    };
    (finding, vec![], vec![])
}

// ---------------------------------------------------------------- 4. hospital definition

fn hospital_assert() -> Assertion {
    Assertion::phrases(&["Hospital means", "in-patient beds"])
}

fn hospital_def(f: &Facts, evidence: &[EvidenceItem]) -> EvalResult {
    let dim = "hospital_definition";
    if evidence.is_empty() {
        return no_evidence(dim, "Could not retrieve the policy's Hospital definition.");
    }
    let top = pick(evidence, |t| {
        contains(t, "hospital means") && contains(t, "in-patient beds")
    });
    let ctx = &f.evidence_context;
    let name = &f.hospital_name;
    let cite = || {
        top.cite(
            "Hospital definition (registration / minimum criteria)",
            Some(hospital_assert()),
        )
    };

    if f.hospital_network_provider {
        let finding = Finding::new(dim, S::SupportsAdmissible, format!("{name} is a network-empanelled provider, which is treated as satisfying the policy's Hospital definition."), 0.75)
            .cite(cite())
            .assume(["Network-empanelled status is treated as proof the facility meets the Hospital definition (the policy text does not state this).".to_string()]);
        return (finding, vec![], vec![]);
    }
    let registered = ctx.get("hospital_registered");
    let criteria = ctx.get("hospital_minimum_criteria_documented");
    if registered == Some(&Value::Bool(false))
        || criteria == Some(&Value::Bool(false))
        || registered == Some(&Value::Null)
    {
        let reason = format!(
            "Supplied evidence does not establish that {name} is registered under the Clinical Establishments Act or meets the policy's minimum-criteria test (round-the-clock qualified nursing and medical staff, minimum in-patient beds, dedicated operation theatre)."
        );
        let finding = Finding::new(dim, S::InsufficientEvidence, reason.clone(), 0.3).cite(cite());
        return (
            finding,
            vec![],
            vec![MissingEvidence::new("hospital_registration_proof", reason)],
        );
    }
    if ctx.is_empty() {
        let reason = format!(
            "{name} is not flagged as a network provider and no registration/minimum-criteria evidence was supplied; the policy's Hospital definition cannot be confirmed from the available documents."
        );
        let finding = Finding::new(dim, S::InsufficientEvidence, reason.clone(), 0.3).cite(cite());
        return (
            finding,
            vec![],
            vec![MissingEvidence::new("hospital_registration_proof", reason)],
        );
    }
    let finding = Finding::new(dim, S::SupportsAdmissible, format!("Available evidence does not contradict {name} meeting the policy's Hospital definition."), 0.5).cite(cite());
    (finding, vec![], vec![])
}

// ---------------------------------------------------------------- 5. domiciliary

fn domiciliary(f: &Facts, evidence: &[EvidenceItem]) -> EvalResult {
    let dim = "domiciliary_treatment_conditions";
    if evidence.is_empty() {
        return no_evidence(
            dim,
            "Could not retrieve the Domiciliary Treatment definition.",
        );
    }
    let top = pick(evidence, |t| {
        contains(t, "removed to a Hospital") && contains(t, "non-availability of room")
    });
    let assertion = || {
        Some(Assertion::phrases(&[
            "removed to a Hospital",
            "non-availability of room",
        ]))
    };
    let finding = match (f.hospital_room_unavailable, f.patient_cannot_be_moved) {
        (room, moved) if room == Some(true) || moved == Some(true) => {
            let reason = if room == Some(true) { "non-availability of a hospital room" } else { "the patient's condition prevents moving them to a Hospital" };
            Finding::new(dim, S::SupportsAdmissible, format!("Domiciliary Treatment condition is met: {reason}."), 0.85)
                .cite(top.cite("Domiciliary Treatment definition condition met", assertion()))
        }
        (Some(false), Some(false)) => Finding::new(
            dim,
            S::SupportsExclusion,
            "Neither Domiciliary Treatment condition (room unavailability or inability to move the patient) is met.",
            0.8,
        )
        .cite(top.cite("Domiciliary Treatment definition condition not met", assertion())),
        _ => return no_evidence(dim, "Case does not state whether a hospital room was unavailable or the patient could not be moved."),
    };
    (finding, vec![], vec![])
}

// ---------------------------------------------------------------- 6. day care

/// Procedure keyword in the case -> the wording used in the policy's named list.
const NAMED_DAYCARE: &[(&str, &str)] = &[
    ("dialysis", "Dialysis"),
    ("chemotherapy", "Chemotherapy"),
    ("radiotherapy", "Radiotherapy"),
    ("eye surgery", "Eye Surgery"),
    ("cataract", "Eye Surgery"),
    ("lithotripsy", "Lithotripsy"),
    ("tonsillectomy", "Tonsillectomy"),
    ("d&c", "D&C"),
    ("dilatation and curettage", "D&C"),
];

fn daycare_applies(f: &Facts) -> bool {
    f.treatment_type == "day_care" || (f.treatment_type == "inpatient" && f.admission_hours < 24.0)
}

fn daycare(f: &Facts, evidence: &[EvidenceItem]) -> EvalResult {
    let dim = "day_care_less_than_24h";
    if evidence.is_empty() {
        return no_evidence(dim, "Could not retrieve the Day Care Treatment definition.");
    }
    let text = f.case_text();
    let named: BTreeSet<&str> = NAMED_DAYCARE
        .iter()
        .filter(|(kw, _)| has_term(&text, kw))
        .map(|(_, w)| *w)
        .collect();
    let definition = find_source(evidence, "less than 24 hrs");
    let def_assert = || {
        Some(Assertion::phrases(&[
            "less than 24 hrs",
            "General or Local Anesthesia",
        ]))
    };

    if !named.is_empty() {
        let terms: Vec<&str> = named.into_iter().collect();
        // Cite the chunk that lists the procedure; if retrieval did not surface it the
        // top chunk is cited and verification flags it, triggering the widened retry.
        let src = pick(evidence, |t| {
            terms
                .iter()
                .any(|term| t.to_lowercase().contains(&term.to_lowercase()))
        });
        let finding = Finding::new(
            dim,
            S::SupportsAdmissible,
            format!("{} is on the policy's named list of treatments covered when completed in under 24 hours.", terms.join(", ")),
            0.85,
        )
        .cite(src.cite("named <24h day-care procedure", Some(Assertion::phrases(&terms))));
        return (finding, vec![], vec![]);
    }
    if f.treatment_type == "day_care" {
        let finding = Finding::new(
            dim,
            S::SupportsAdmissible,
            "Treatment is classified as day care; admissibility assumes it required anesthesia and would otherwise have needed >24h hospitalization, per the Day Care Treatment definition.",
            0.6,
        )
        .cite(definition.cite("Day Care Treatment definition", def_assert()))
        .assume(["The procedure is assumed to need anesthesia and to have otherwise required a stay over 24 hours (not stated in the case).".to_string()]);
        return (finding, vec![], vec![]);
    }
    let statement = format!(
        "Admission was only {} hours for a procedure not on the policy's named day-care list; medical necessity for a stay under 24 hours cannot be confirmed from the supplied evidence.",
        fmt_hours(f.admission_hours)
    );
    let finding = Finding::new(dim, S::InsufficientEvidence, statement.clone(), 0.3)
        .cite(definition.cite("Day Care Treatment definition", def_assert()));
    (
        finding,
        vec![],
        vec![MissingEvidence::new(
            "day_care_medical_justification",
            statement,
        )],
    )
}

fn fmt_hours(h: f64) -> String {
    if (h - h.round()).abs() < 1e-9 {
        format!("{}", h.round() as i64)
    } else {
        format!("{h}")
    }
}

// ---------------------------------------------------------------- 7. pre/post window

fn pre_post(f: &Facts, evidence: &[EvidenceItem]) -> EvalResult {
    let dim = "pre_post_hospitalization_window";
    if evidence.is_empty() {
        return no_evidence(
            dim,
            "Could not retrieve the pre/post-hospitalization time-window clause.",
        );
    }
    let top = pick(evidence, |t| {
        matches!(rx::extract_pre_post_windows(t), (Some(_), Some(_)))
    });
    let (Some(pre_limit), Some(post_limit)) = rx::extract_pre_post_windows(&top.text) else {
        return no_evidence(
            dim,
            "Retrieved clause did not contain parseable pre/post-hospitalization day limits.",
        );
    };
    let window = || {
        Some(Assertion::threshold(
            &[(pre_limit, "days"), (post_limit, "days")],
            &[],
        ))
    };
    let claim = format!("{pre_limit}-day pre / {post_limit}-day post hospitalization window");

    let Some(timing) = f.expense_timing.as_ref() else {
        // Missing timing metadata alone is not a reason to withhold payment; only a
        // confirmed window violation is.
        let reason = format!(
            "Pre/post-hospitalization expenses were claimed but exact admission/discharge-relative timing was not supplied; assumed within the {pre_limit}-day/{post_limit}-day windows pending documentation."
        );
        let finding = Finding::new(dim, S::SupportsAdmissible, reason.clone(), 0.55)
            .cite(top.cite(claim, window()))
            .assume([format!("Pre/post-hospitalization expenses are assumed to fall inside the {pre_limit}/{post_limit}-day windows (timing not supplied).")]);
        return (
            finding,
            vec![],
            vec![MissingEvidence::new("expense_timing", reason)],
        );
    };
    let pre_days = timing
        .get("pre_hospitalization_days_before_admission")
        .and_then(Value::as_i64);
    let post_days = timing
        .get("post_hospitalization_days_after_discharge")
        .and_then(Value::as_i64);
    let same_condition = timing
        .get("same_condition_confirmed")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let pre_ok = pre_days.is_none_or(|d| d <= pre_limit);
    let post_ok = post_days.is_none_or(|d| d <= post_limit);
    let show = |d: Option<i64>| {
        d.map(|v| v.to_string())
            .unwrap_or_else(|| "unstated".into())
    };

    if pre_ok && post_ok && same_condition {
        let finding = Finding::new(
            dim,
            S::SupportsAdmissible,
            format!(
                "Pre-hospitalization ({} days) and post-hospitalization ({} days) expenses are within the {pre_limit}/{post_limit}-day windows.",
                show(pre_days),
                show(post_days)
            ),
            0.85,
        )
        .cite(top.cite(claim, window()));
        return (finding, vec![], vec![]);
    }
    let mut deduction = 0.0;
    if !pre_ok {
        deduction += f.expense("pre_hospitalization");
    }
    if !post_ok {
        deduction += f.expense("post_hospitalization");
    }
    if !same_condition {
        deduction = f.expense("pre_hospitalization") + f.expense("post_hospitalization");
    }
    let finding = Finding::new(
        dim,
        S::SupportsLimit,
        format!("Pre/post-hospitalization expenses fall outside the {pre_limit}-day/{post_limit}-day windows or are not confirmed to relate to the same condition, so they are not payable."),
        0.75,
    )
    .cite(top.cite(claim, window()));
    let mut limits = vec![];
    if deduction > 0.0 {
        limits.push(ApplicableLimit {
            description: "Pre/post-hospitalization expenses outside policy time window".into(),
            dimension: dim.into(),
            deduction_inr: Some(deduction),
            citations: vec![top.cite("time-window limit", window())],
        });
    }
    (finding, limits, vec![])
}

// ---------------------------------------------------------------- 8. cosmetic

const COSMETIC: &[&str] = &["cosmetic", "aesthetic", "plastic surgery"];
const INJURY_CONTEXT: &[&str] = &["accident", "injury", "burn", "trauma", "reconstruct"];

fn cosmetic_applies(f: &Facts) -> bool {
    let text = f.case_text();
    COSMETIC.iter().any(|k| has_term(&text, k))
}

fn cosmetic(f: &Facts, evidence: &[EvidenceItem]) -> EvalResult {
    let dim = "cosmetic_exclusion";
    if evidence.is_empty() {
        return no_evidence(
            dim,
            "Could not retrieve the cosmetic/aesthetic treatment exclusion clause.",
        );
    }
    let top = pick(evidence, |t| {
        contains(t, "cosmetic") && contains(t, "plastic surgery")
    });
    let text = f.case_text();
    if INJURY_CONTEXT.iter().any(|k| has_term(&text, k)) {
        return no_evidence(dim, "Diagnosis/procedure mentions cosmetic terms alongside an injury context; cannot confirm from supplied evidence whether the injury/disease carve-out applies.");
    }
    let finding = Finding::new(
        dim,
        S::SupportsExclusion,
        "Diagnosis/procedure is cosmetic/aesthetic in nature with no stated injury or disease basis, which the policy excludes.",
        0.85,
    )
    .cite(top.cite("cosmetic/aesthetic treatment exclusion", Some(Assertion::phrases(&["cosmetic", "plastic surgery"]))));
    (finding, vec![], vec![])
}

// ---------------------------------------------------------------- 9. experimental

fn experimental(_f: &Facts, evidence: &[EvidenceItem]) -> EvalResult {
    let dim = "experimental_unproven_treatment";
    if evidence.is_empty() {
        return no_evidence(
            dim,
            "Could not retrieve the Unproven/Experimental Treatment definition.",
        );
    }
    let top = find_source(evidence, "Unproven/Experimental Treatment");
    let reason = "The policy defines 'Unproven/Experimental Treatment' but does not contain an explicit numbered exclusion clause naming it; the nearest analogous exclusion (treatments not approved by the Indian Medical Council / non-allopathic treatment) does not unambiguously cover an experimental allopathic therapy. A safe decision cannot be made from the supplied policy text alone.";
    let finding = Finding::new(dim, S::InsufficientEvidence, reason, 0.3).cite(top.cite(
        "Unproven/Experimental Treatment is defined but no other clause of the policy mentions it",
        Some(Assertion::Absence {
            term: "experimental".into(),
            allowed_chunk_ids: vec![top.chunk_id.clone()],
        }),
    ));
    (
        finding,
        vec![],
        vec![MissingEvidence::new(
            "experimental_treatment_exclusion_basis",
            reason,
        )],
    )
}

// ---------------------------------------------------------------- 10. sub-limits

#[allow(clippy::too_many_arguments)]
fn limit_assert(
    keyword: &str,
    percent: Option<f64>,
    flat: Option<f64>,
    si: f64,
    multiplier: f64,
    claimed: f64,
    cap: f64,
    deduction: f64,
) -> Assertion {
    Assertion::Limit {
        keyword: keyword.into(),
        percent,
        flat,
        sum_insured: Some(si),
        multiplier,
        claimed: Some(claimed),
        cap: Some(cap),
        deduction: Some(deduction),
    }
}

#[allow(clippy::needless_late_init)]
fn sub_limits(f: &Facts, evidence: &[EvidenceItem]) -> EvalResult {
    let dim = "category_sub_limits";
    if evidence.is_empty() {
        return no_evidence(dim, "Could not retrieve the coverage sub-limit clauses.");
    }
    let combined: String = evidence
        .iter()
        .map(|e| e.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let si = f.sum_insured;
    let mut limits: Vec<ApplicableLimit> = vec![];
    let mut assumptions: Vec<String> = vec![];

    let mut add_limit = |description: String,
                         keyword: &str,
                         claimed: f64,
                         cap: f64,
                         percent: Option<f64>,
                         flat: Option<f64>,
                         multiplier: f64| {
        // Cite the chunk whose clause states the limit, not merely the first mention.
        let src = evidence
            .iter()
            .find(|e| rx::clause_with(&e.text, keyword).is_some())
            .unwrap_or_else(|| find_source(evidence, keyword));
        let deduction = claimed - cap;
        let short = description
            .split(" (")
            .next()
            .unwrap_or(&description)
            .to_string();
        limits.push(ApplicableLimit {
            description,
            dimension: dim.into(),
            deduction_inr: Some(deduction),
            citations: vec![src.cite(
                short,
                Some(limit_assert(
                    keyword, percent, flat, si, multiplier, claimed, cap, deduction,
                )),
            )],
        });
    };

    let statement;
    if f.treatment_type == "domiciliary" {
        if let Some(pct) = rx::extract_percent_of_si(&combined, "Domiciliary") {
            let cap = pct / 100.0 * si;
            let claimed =
                f.expense("doctor_fees") + f.expense("medicines_diagnostics") + f.expense("room");
            if claimed > cap {
                add_limit(
                    format!("Domiciliary Hospitalization aggregate sub-limit ({}% of Basic Sum Insured = INR {})", fmt_pct(pct), fmt_inr(cap)),
                    "Domiciliary",
                    claimed,
                    cap,
                    Some(pct),
                    None,
                    1.0,
                );
            }
        }
        statement = "Domiciliary Hospitalization is capped at an aggregate percentage of the Basic Sum Insured.";
    } else {
        let los_days = ((f.admission_hours.max(0.0).trunc() as i64 + 23) / 24).max(1);
        let room = f.expense("room");
        let room_pct = rx::extract_percent_of_si(&combined, "Room");
        if let Some(pct) = room_pct {
            if room > 0.0 {
                assumptions.push(format!("The 'Normal Room' limit is applied per day for the length of stay ({los_days} day(s), admission hours rounded up); the room clause does not say 'per day' explicitly (the ICU clause does)."));
                assumptions.push("All claimed room expense is treated as normal-room rent; the case does not split out ICU days.".into());
                let cap = pct / 100.0 * si * los_days as f64;
                if room > cap {
                    add_limit(
                        format!("Room/boarding/nursing sub-limit ({}% of Basic Sum Insured per day x {los_days} day(s) = INR {})", fmt_pct(pct), fmt_inr(cap)),
                        "Room",
                        room,
                        cap,
                        Some(pct),
                        None,
                        los_days as f64,
                    );
                }
            }
        }
        let fees = f.expense("doctor_fees");
        let mut fees_deduction = 0.0;
        if let Some(pct) = rx::extract_percent_of_si(&combined, "fees") {
            let cap = pct / 100.0 * si;
            if fees > 0.0 && fees > cap {
                fees_deduction = fees - cap;
                add_limit(
                    format!(
                        "Medical practitioner/surgeon fees sub-limit ({}% of Sum Insured = INR {})",
                        fmt_pct(pct),
                        fmt_inr(cap)
                    ),
                    "fees",
                    fees,
                    cap,
                    Some(pct),
                    None,
                    1.0,
                );
            }
        }
        let meds = f.expense("medicines_diagnostics");
        let mut meds_deduction = 0.0;
        if let Some(pct) = rx::extract_percent_of_si(&combined, "Anesthesia") {
            let cap = pct / 100.0 * si;
            if meds > 0.0 && meds > cap {
                meds_deduction = meds - cap;
                add_limit(
                    format!(
                        "Anesthesia/medicines/diagnostics sub-limit ({}% of Sum Insured = INR {})",
                        fmt_pct(pct),
                        fmt_inr(cap)
                    ),
                    "Anesthesia",
                    meds,
                    cap,
                    Some(pct),
                    None,
                    1.0,
                );
            }
        }
        // The "Any One Illness" cap applies to agreed package charges only.
        let package_pct = rx::extract_percent_of_si(&combined, "package");
        match package_pct {
            Some(pct) if f.package_charges_agreed => {
                let package_cap = pct / 100.0 * si;
                let room_after = room_pct.map(|p| room.min(p / 100.0 * si * los_days as f64)).unwrap_or(room);
                let subtotal = room_after + (fees - fees_deduction) + (meds - meds_deduction);
                if subtotal > package_cap {
                    add_limit(format!("'Any One Illness' package cap ({}% of Sum Insured = INR {})", fmt_pct(pct), fmt_inr(package_cap)), "package", subtotal, package_cap, Some(pct), None, 1.0);
                }
            }
            Some(pct) => assumptions.push(format!("No agreed package charges are stated in the case, so the {}% 'Any One Illness' package cap is not applied.", fmt_pct(pct))),
            None => {}
        }
        let amb = f.expense("ambulance");
        let amb_pct = rx::extract_percent_of_si(&combined, "Ambulance");
        let amb_flat = rx::extract_flat_amount(&combined, "Ambulance");
        if amb > 0.0 && (amb_pct.is_some() || amb_flat.is_some()) {
            let cap = [amb_pct.map(|p| p / 100.0 * si), amb_flat]
                .into_iter()
                .flatten()
                .fold(f64::INFINITY, f64::min);
            if amb > cap {
                let basis = match (amb_pct, amb_flat) {
                    (Some(p), Some(fl)) => format!(
                        "lower of {}% of Basic Sum Insured and INR {}",
                        fmt_pct(p),
                        fmt_inr(fl)
                    ),
                    (Some(p), None) => format!("{}% of Basic Sum Insured", fmt_pct(p)),
                    (None, Some(fl)) => format!("flat INR {}", fmt_inr(fl)),
                    (None, None) => unreachable!(),
                };
                add_limit(
                    format!(
                        "Ambulance charges sub-limit ({basis} = INR {})",
                        fmt_inr(cap)
                    ),
                    "Ambulance",
                    amb,
                    cap,
                    amb_pct,
                    amb_flat,
                    1.0,
                );
            }
        }
        statement = "Category-wise sub-limits (room/boarding, practitioner fees, medicines/anesthesia, ambulance) were checked against claimed expenses.";
    }

    // The whole payout is subject to the overall Sum Insured limit.
    let total: f64 = f.expenses.values().fold(0.0, |a, b| a + b);
    let remaining = total
        - limits
            .iter()
            .map(|l| l.deduction_inr.unwrap_or(0.0))
            .sum::<f64>();
    if si > 0.0 && remaining > si {
        if let Some(overall) = evidence
            .iter()
            .find(|e| norm(&e.text).contains("overall sum insured"))
        {
            limits.push(ApplicableLimit {
                description: format!(
                    "Overall Sum Insured limit (INR {}) caps the total payable",
                    fmt_inr(si)
                ),
                dimension: dim.into(),
                deduction_inr: Some(remaining - si),
                citations: vec![overall.cite(
                    "Overall Sum Insured limit",
                    Some(Assertion::phrases(&["overall sum insured"])),
                )],
            });
        } else {
            assumptions.push("The total payable exceeds the sum insured but the overall-limit clause was not retrieved, so the cap was not applied.".into());
        }
    }

    let has_limits = !limits.is_empty();
    let citations: Vec<_> = limits
        .iter()
        .flat_map(|l| l.citations.iter().cloned())
        .take(1)
        .collect();
    let mut finding = Finding::new(
        dim,
        if has_limits {
            S::SupportsLimit
        } else {
            S::NotApplicable
        },
        format!(
            "{statement}{}",
            if has_limits {
                format!(" {} limit(s) reduce the payable amount.", limits.len())
            } else {
                " No category sub-limit is exceeded.".into()
            }
        ),
        if has_limits { 0.8 } else { 0.6 },
    );
    finding.applicable = has_limits;
    finding.citations = if citations.is_empty() {
        vec![evidence[0].cite("coverage sub-limits", None)]
    } else {
        citations
    };
    if has_limits {
        finding.assumptions = assumptions;
    }
    (finding, limits, vec![])
}

// ---------------------------------------------------------------- 11. unmodelled risk
//
// The ten dimensions above cover the rules this system models. The policy has other
// specific exclusions (dental, pregnancy, spectacles, HIV, ...). Rather than silently
// ignoring them, this check looks at what the case is about and flags any exclusion
// that names the same thing but is not already used by a modelled dimension. It never
// decides the claim; it abstains and hands the reviewer the exact clause.

const GENERIC_TERMS: &[&str] = &[
    "acute",
    "chronic",
    "severe",
    "mild",
    "treatment",
    "surgery",
    "surgical",
    "procedure",
    "therapy",
    "condition",
    "conditions",
    "complication",
    "complications",
    "disorder",
    "disease",
    "diseases",
    "requiring",
    "home",
    "medical",
    "management",
    "hospital",
    "hospitalization",
    "with",
    "from",
    "that",
    "this",
    "other",
    "similar",
    "related",
    "exacerbation",
    "repair",
    "existing",
    "experimental",
    "unproven",
    "infection",
    "observation",
    "general",
    "domiciliary",
    "day",
    "care",
    "inpatient",
    "admission",
    // anatomical / positional qualifiers carry no diagnostic subject ("internal fixation")
    "internal",
    "external",
    "left",
    "right",
    "bilateral",
    "unilateral",
    "upper",
    "lower",
    "anterior",
    "posterior",
    "partial",
    "total",
    "primary",
    "secondary",
];

/// Phrases identifying clauses whose subject a modelled dimension already evaluates
/// (whether or not that dimension applied to this case). Such a clause is not an
/// "unmodelled" risk: the modelled rule's applicability check has already decided.
const OWNED_CLAUSE_SIGNATURES: &[&str] = &[
    "first year of operation",
    "months of continuous coverage",
    "cosmetic or aesthetic",
    "first 30 days",
];

fn owned_by_modelled_rule(text: &str) -> bool {
    let n = norm(text);
    OWNED_CLAUSE_SIGNATURES.iter().any(|s| n.contains(s))
        || rx::extract_waiting_days(text).is_some()
}

static WORD4: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z]{4,}").expect("regex"));

fn stem_set(text: &str) -> BTreeSet<String> {
    WORD4
        .find_iter(&text.to_lowercase())
        .map(|m| m.as_str().trim_end_matches('s').to_string())
        .collect()
}

pub fn case_terms(text: &str) -> BTreeSet<String> {
    let generic: BTreeSet<String> = GENERIC_TERMS
        .iter()
        .map(|g| g.trim_end_matches('s').to_string())
        .collect();
    stem_set(text)
        .into_iter()
        .filter(|w| !generic.contains(w))
        .collect()
}

fn unmodelled(f: &Facts, evidence: &[EvidenceItem]) -> EvalResult {
    let dim = UNMODELLED;
    let terms = case_terms(&format!("{} {}", f.diagnosis, f.procedure));
    let handled: BTreeSet<&str> = f.cited_chunk_ids.iter().map(String::as_str).collect();
    for ev in evidence {
        if ev.section != "What We Exclude"
            || handled.contains(ev.chunk_id.as_str())
            || owned_by_modelled_rule(&ev.text)
        {
            continue;
        }
        let body = stem_set(&ev.text);
        if let Some(hit) = terms.intersection(&body).next() {
            let statement = format!(
                "The diagnosis/procedure mentions '{hit}', which the policy names in a specific exclusion ({}, page {}). No modelled rule evaluates it, so the system will not decide this on its own; a reviewer should confirm whether the exclusion applies.",
                ev.label, ev.page_start
            );
            let finding =
                Finding::new(dim, S::InsufficientEvidence, statement.clone(), 0.3).cite(ev.cite(
                    "Possible exclusion not covered by any modelled check",
                    Some(Assertion::phrases_owned(vec![hit.clone()])),
                ));
            return (finding, vec![], vec![MissingEvidence::new(dim, statement)]);
        }
    }
    let mut finding = Finding::new(
        dim,
        S::NotApplicable,
        "No unmodelled policy exclusion matches the diagnosis or procedure.",
        0.6,
    );
    finding.applicable = false;
    (finding, vec![], vec![])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_terms_drop_generic_words() {
        let t = case_terms("Acute asthma exacerbation requiring home treatment");
        assert!(t.contains("asthma"));
        assert!(!t.contains("acute") && !t.contains("treatment"));
    }

    #[test]
    fn named_disease_detection() {
        let f = Facts {
            treatment_type: "inpatient".into(),
            diagnosis: "Inguinal hernia".into(),
            ..Default::default()
        };
        assert!(named_disease_applies(&f));
        let g = Facts {
            treatment_type: "inpatient".into(),
            diagnosis: "Cholecystectomy".into(),
            ..Default::default()
        };
        assert!(!named_disease_applies(&g));
    }
}
