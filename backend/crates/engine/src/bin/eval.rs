//! `claim-eval`: run every bundled case through the full pipeline (offline,
//! deterministic) and print a metrics report. Exits non-zero on regression so it
//! can gate CI.
//!
//! Usage: `cargo run -p claim-engine --bin claim-eval [-- --json]`

use claim_engine::eval::{bundled_cases, run_suite};
use claim_engine::Engine;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let json = args.iter().any(|a| a == "--json");
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    let engine = Engine::bundled();

    // `--case ID` prints the full decision contract for one bundled case.
    if let Some(pos) = args.iter().position(|a| a == "--case") {
        let id = args.get(pos + 1).cloned().unwrap_or_default();
        let case = bundled_cases()
            .into_iter()
            .find(|c| c.get("case_id").and_then(|v| v.as_str()) == Some(id.as_str()));
        let Some(case) = case else {
            eprintln!("unknown case {id}");
            std::process::exit(2);
        };
        let state =
            rt.block_on(engine.analyze_unchecked(case, &claim_engine::OfflineLlm, Some(false)));
        println!(
            "{}",
            serde_json::to_string_pretty(&state.to_response()).expect("serialise")
        );
        return;
    }
    let report = rt.block_on(run_suite(&engine, &bundled_cases()));

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("serialise")
        );
    } else {
        println!("Engine            {}", report.engine_version);
        println!(
            "Decision accuracy {}/{} ({:.1}%), unsafe decisions: {}",
            report.correct,
            report.labelled_cases,
            report.accuracy * 100.0,
            report.unsafe_decisions
        );
        println!(
            "Abstentions       {}/{} required NEEDS_REVIEW caught",
            report.abstentions_caught, report.required_abstentions
        );
        println!(
            "Amounts           {}/{} exact deduction matches",
            report.amount_exact, report.amount_cases
        );
        println!(
            "Claims verified   {}/{}",
            report.claims_verified, report.claims_total
        );
        println!(
            "Fault injection   {}/{} detected",
            report.faults_detected, report.faults_injected
        );
        println!("Deterministic     {}", report.deterministic);
        println!(
            "Latency           p50 {:.2} ms, p95 {:.2} ms",
            report.latency_p50_ms, report.latency_p95_ms
        );
        println!();
        println!(
            "{:<10} {:<24} {:<24} {:>12} {:>12}",
            "case", "expected", "actual", "exp. ded.", "deduction"
        );
        for c in &report.cases {
            let mark = match c.correct {
                Some(true) => " ",
                Some(false) => "x",
                None => "?",
            };
            println!(
                "{mark}{:<9} {:<24} {:<24} {:>12} {:>12.0}",
                c.case_id,
                c.expected
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "-".into()),
                c.actual.to_string(),
                c.expected_deduction_inr
                    .map(|d| format!("{d:.0}"))
                    .unwrap_or_else(|| "-".into()),
                c.actual_deduction_inr
            );
        }
    }
    let ok = report.correct == report.labelled_cases
        && report.unsafe_decisions == 0
        && report.abstentions_caught == report.required_abstentions
        && report.amount_exact == report.amount_cases
        && report.faults_detected == report.faults_injected
        && report.deterministic;
    if !ok {
        eprintln!("evaluation gate FAILED");
        std::process::exit(1);
    }
}
