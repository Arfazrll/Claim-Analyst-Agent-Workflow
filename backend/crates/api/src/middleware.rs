//! Cross-cutting HTTP middleware: client IP, rate limiting, Origin checks, security
//! headers and request metrics.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{ConnectInfo, MatchedPath, State};
use axum::http::{HeaderMap, HeaderValue, Method, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::auth::{verify_access, ACCESS_COOKIE};
use crate::error::ApiError;
use crate::state::AppState;

/// Client IP: the proxy-provided `X-Real-IP` when `TRUST_PROXY=true` (nginx sets it
/// from the TCP peer, so it cannot be spoofed by clients), else the socket peer.
pub fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>, trust_proxy: bool) -> String {
    if trust_proxy {
        if let Some(ip) = headers
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .filter(|v| v.parse::<std::net::IpAddr>().is_ok())
        {
            return ip.to_string();
        }
        if let Some(first) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
        {
            let first = first.trim();
            if first.parse::<std::net::IpAddr>().is_ok() {
                return first.to_string();
            }
        }
    }
    peer.map(|p| p.ip().to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub fn ip_of(req: &Request<Body>, trust_proxy: bool) -> String {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    client_ip(req.headers(), peer, trust_proxy)
}

fn bucket_for(method: &Method, path: &str) -> Option<&'static str> {
    if path.starts_with("/api/auth/login")
        || path.starts_with("/api/auth/register")
        || path.starts_with("/api/auth/refresh")
        || path.starts_with("/api/auth/password")
    {
        Some("auth")
    } else if *method == Method::POST
        && (path == "/api/claims" || path == "/api/analyze" || path.ends_with("/reanalyze"))
    {
        Some("analyze")
    } else if path.starts_with("/api/") {
        Some("api")
    } else {
        None
    }
}

/// Sliding-window limits per bucket. Analysis is keyed per user when signed in (so
/// users behind one NAT do not share a budget), everything else per client IP.
pub async fn rate_limit(State(state): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let Some(bucket) = bucket_for(req.method(), &path) else {
        return next.run(req).await;
    };
    let ip = ip_of(&req, state.cfg.trust_proxy);
    let (limit, identity) = match bucket {
        "auth" => (state.cfg.rate_auth_per_min, ip.clone()),
        "analyze" => {
            let user = axum_extra::extract::cookie::CookieJar::from_headers(req.headers())
                .get(ACCESS_COOKIE)
                .and_then(|c| verify_access(&state.cfg, c.value()))
                .map(|c| format!("user:{}", c.sub));
            (
                state.cfg.rate_analyze_per_min,
                user.unwrap_or_else(|| ip.clone()),
            )
        }
        _ => (state.cfg.rate_api_per_min, ip.clone()),
    };
    let d = state
        .cache
        .rate_limit(bucket, &identity, limit, Duration::from_secs(60))
        .await;
    if !d.allowed {
        state
            .metrics
            .rate_limited
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tracing::warn!(bucket, identity = %identity, "rate limited");
        let mut resp = ApiError::RateLimited {
            retry_after_secs: d.reset_secs,
        }
        .into_response();
        set_rate_headers(resp.headers_mut(), d.limit, 0, d.reset_secs);
        return resp;
    }
    let mut resp = next.run(req).await;
    set_rate_headers(resp.headers_mut(), d.limit, d.remaining, d.reset_secs);
    resp
}

fn set_rate_headers(h: &mut HeaderMap, limit: u64, remaining: u64, reset: u64) {
    for (k, v) in [
        ("ratelimit-limit", limit),
        ("ratelimit-remaining", remaining),
        ("ratelimit-reset", reset),
    ] {
        if let Ok(val) = HeaderValue::from_str(&v.to_string()) {
            h.insert(k, val);
        }
    }
}

/// Reject cross-site unsafe requests whose `Origin` is not on the allow-list (defence
/// in depth on top of SameSite=Strict cookies and the CSRF token).
pub async fn origin_guard(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if matches!(
        *req.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) {
        if let Some(origin) = req.headers().get("origin").and_then(|v| v.to_str().ok()) {
            let origin = origin.trim_end_matches('/');
            let same_host = req
                .headers()
                .get("host")
                .and_then(|h| h.to_str().ok())
                .is_some_and(|h| origin.ends_with(&format!("://{h}")));
            if !same_host && !state.cfg.cors_origins.iter().any(|o| o == origin) {
                tracing::warn!(origin, "blocked cross-origin request");
                return ApiError::Forbidden("Cross-origin request blocked.".into()).into_response();
            }
        }
    }
    next.run(req).await
}

pub async fn security_headers(req: Request<Body>, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    for (k, v) in [
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=(), payment=()",
        ),
        (
            "content-security-policy",
            "default-src 'none'; frame-ancestors 'none'",
        ),
        (
            "strict-transport-security",
            "max-age=63072000; includeSubDomains",
        ),
    ] {
        h.insert(k, HeaderValue::from_static(v));
    }
    if !h.contains_key("cache-control") {
        h.insert("cache-control", HeaderValue::from_static("no-store"));
    }
    resp
}

pub async fn track(State(state): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "unmatched".into());
    let started = Instant::now();
    let resp = next.run(req).await;
    state.metrics.observe(
        &route,
        resp.status().as_u16(),
        started.elapsed().as_millis() as u64,
    );
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ip_ignores_spoofed_headers_without_trust() {
        let mut h = HeaderMap::new();
        h.insert("x-real-ip", HeaderValue::from_static("1.2.3.4"));
        let peer: SocketAddr = "10.0.0.9:5000".parse().unwrap();
        assert_eq!(client_ip(&h, Some(peer), false), "10.0.0.9");
        assert_eq!(client_ip(&h, Some(peer), true), "1.2.3.4");
        h.insert("x-real-ip", HeaderValue::from_static("not-an-ip"));
        assert_eq!(client_ip(&h, Some(peer), true), "10.0.0.9");
    }

    #[test]
    fn buckets() {
        assert_eq!(bucket_for(&Method::POST, "/api/auth/login"), Some("auth"));
        assert_eq!(bucket_for(&Method::POST, "/api/claims"), Some("analyze"));
        assert_eq!(bucket_for(&Method::GET, "/api/claims"), Some("api"));
        assert_eq!(bucket_for(&Method::GET, "/healthz"), None);
    }
}
