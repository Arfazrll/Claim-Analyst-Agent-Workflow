//! Prometheus text-format metrics (no extra dependency).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Default)]
pub struct Metrics {
    requests: Mutex<BTreeMap<(String, u16), u64>>,
    latency_ms_sum: AtomicU64,
    latency_count: AtomicU64,
    pub analyses: AtomicU64,
    pub analysis_cache_hits: AtomicU64,
    pub rate_limited: AtomicU64,
    pub login_failures: AtomicU64,
    decisions: Mutex<BTreeMap<String, u64>>,
}

impl Metrics {
    pub fn observe(&self, route: &str, status: u16, ms: u64) {
        if let Ok(mut m) = self.requests.lock() {
            *m.entry((route.to_string(), status)).or_default() += 1;
        }
        self.latency_ms_sum.fetch_add(ms, Ordering::Relaxed);
        self.latency_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn decision(&self, d: &str) {
        if let Ok(mut m) = self.decisions.lock() {
            *m.entry(d.to_string()).or_default() += 1;
        }
    }

    pub fn render(&self, extra: &[(&str, &str, f64)]) -> String {
        let mut out = String::new();
        out.push_str("# HELP caw_http_requests_total HTTP requests by route and status.\n# TYPE caw_http_requests_total counter\n");
        if let Ok(m) = self.requests.lock() {
            for ((route, status), n) in m.iter() {
                out.push_str(&format!(
                    "caw_http_requests_total{{route=\"{route}\",status=\"{status}\"}} {n}\n"
                ));
            }
        }
        out.push_str("# TYPE caw_http_request_duration_ms_sum counter\n");
        out.push_str(&format!(
            "caw_http_request_duration_ms_sum {}\n",
            self.latency_ms_sum.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "caw_http_request_duration_ms_count {}\n",
            self.latency_count.load(Ordering::Relaxed)
        ));
        for (name, help, v) in [
            (
                "caw_analyses_total",
                "Claim analyses executed.",
                self.analyses.load(Ordering::Relaxed) as f64,
            ),
            (
                "caw_analysis_cache_hits_total",
                "Analyses served from cache.",
                self.analysis_cache_hits.load(Ordering::Relaxed) as f64,
            ),
            (
                "caw_rate_limited_total",
                "Requests rejected by the rate limiter.",
                self.rate_limited.load(Ordering::Relaxed) as f64,
            ),
            (
                "caw_login_failures_total",
                "Failed sign-in attempts.",
                self.login_failures.load(Ordering::Relaxed) as f64,
            ),
        ] {
            out.push_str(&format!(
                "# HELP {name} {help}\n# TYPE {name} counter\n{name} {v}\n"
            ));
        }
        out.push_str("# TYPE caw_decisions_total counter\n");
        if let Ok(m) = self.decisions.lock() {
            for (d, n) in m.iter() {
                out.push_str(&format!("caw_decisions_total{{decision=\"{d}\"}} {n}\n"));
            }
        }
        for (name, help, v) in extra {
            out.push_str(&format!(
                "# HELP {name} {help}\n# TYPE {name} gauge\n{name} {v}\n"
            ));
        }
        out
    }
}
