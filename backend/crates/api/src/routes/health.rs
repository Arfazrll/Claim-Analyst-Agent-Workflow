use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::crypto::constant_time_eq;
use crate::state::AppState;

/// Liveness: the process is up.
pub async fn live(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({
        "status": "ok", "version": env!("CARGO_PKG_VERSION"), "engine": claim_engine::ENGINE_VERSION,
        "instance": state.instance_id, "uptime_s": state.started_at.elapsed().as_secs(),
    }))
}

/// Readiness: dependencies reachable, so the load balancer may route traffic here.
pub async fn ready(State(state): State<AppState>) -> Response {
    let db = state.db.ping().await;
    let cache = state.cache.ping().await;
    let ok = db && cache;
    let body = Json(json!({
        "status": if ok { "ready" } else { "degraded" }, "database": db, "database_kind": state.db.kind,
        "cache": state.cache.backend(), "cache_ok": cache, "policy_chunks": state.engine.retriever().len(),
        "llm_provider": state.llm.provider(),
    }));
    if ok {
        body.into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, body).into_response()
    }
}

pub async fn metrics(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = &state.cfg.metrics_token {
        let given = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        if !constant_time_eq(given, token) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    let s = &state.cache.stats;
    let ord = std::sync::atomic::Ordering::Relaxed;
    let body = state.metrics.render(&[
        ("caw_cache_hits", "Cache hits.", s.hits.load(ord) as f64),
        (
            "caw_cache_misses",
            "Cache misses.",
            s.misses.load(ord) as f64,
        ),
        (
            "caw_redis_errors",
            "Redis errors (fell back to local cache).",
            s.redis_errors.load(ord) as f64,
        ),
        (
            "caw_uptime_seconds",
            "Process uptime.",
            state.started_at.elapsed().as_secs() as f64,
        ),
    ]);
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response()
}
