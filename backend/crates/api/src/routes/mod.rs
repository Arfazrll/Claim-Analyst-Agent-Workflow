//! Router assembly and the middleware stack.

pub mod admin;
pub mod auth;
pub mod claims;
pub mod health;
pub mod ledger;
pub mod policy;

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{header, HeaderName, HeaderValue, Method};
use axum::routing::{get, patch, post};
use axum::{middleware as mw, Router};
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::sensitive_headers::SetSensitiveRequestHeadersLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::middleware::{client_ip, origin_guard, rate_limit, security_headers, track};
use crate::state::AppState;

/// Client IP extractor that never fails (falls back to "unknown").
pub struct ClientIp(pub String);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0);
        Ok(ClientIp(client_ip(
            &parts.headers,
            peer,
            state.cfg.trust_proxy,
        )))
    }
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/refresh", post(auth::refresh))
        .route("/auth/me", get(auth::me))
        .route("/auth/password", post(auth::change_password))
        .route("/analyze", post(claims::analyze_stateless))
        .route("/claims", get(claims::list).post(claims::create))
        .route("/claims/{id}", get(claims::get_one))
        .route("/claims/{id}/review", post(claims::review))
        .route("/claims/{id}/reanalyze", post(claims::reanalyze))
        .route("/claims/{id}/proof", get(claims::proof))
        .route("/claims/{id}/export", get(claims::export))
        .route("/samples", get(claims::samples))
        .route("/stats", get(claims::stats))
        .route("/policy/chunks", get(policy::chunks))
        .route("/policy/chunks/{id}", get(policy::chunk))
        .route("/policy/search", get(policy::search))
        .route("/policy/dimensions", get(policy::dimensions))
        .route("/ledger/status", get(ledger::status))
        .route("/ledger/entries", get(ledger::entries))
        .route("/ledger/verify", get(ledger::verify))
        .route("/ledger/requeue", post(ledger::requeue))
        .route("/admin/users", get(admin::users))
        .route("/admin/users/{id}", patch(admin::update_user))
        .route("/admin/audit", get(admin::audit))
        .route("/admin/eval", post(admin::eval));

    let origins: Vec<HeaderValue> = state
        .cfg
        .cors_origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_credentials(true)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::OPTIONS])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            HeaderName::from_static("x-csrf-token"),
            HeaderName::from_static("idempotency-key"),
        ])
        .expose_headers([
            HeaderName::from_static("ratelimit-remaining"),
            HeaderName::from_static("x-request-id"),
            HeaderName::from_static("x-cache"),
        ])
        .max_age(std::time::Duration::from_secs(600));
    let request_id = HeaderName::from_static("x-request-id");

    Router::new()
        .nest("/api", api)
        .route("/healthz", get(health::live))
        .route("/readyz", get(health::ready))
        .route("/metrics", get(health::metrics))
        .layer(DefaultBodyLimit::max(state.cfg.body_limit_bytes))
        .layer(mw::from_fn_with_state(state.clone(), track))
        .layer(mw::from_fn_with_state(state.clone(), rate_limit))
        .layer(mw::from_fn_with_state(state.clone(), origin_guard))
        .layer(mw::from_fn(security_headers))
        .layer(
            ServiceBuilder::new()
                .layer(CatchPanicLayer::new())
                .layer(SetRequestIdLayer::new(request_id.clone(), MakeRequestUuid))
                .layer(PropagateRequestIdLayer::new(request_id))
                .layer(SetSensitiveRequestHeadersLayer::new([
                    header::AUTHORIZATION,
                    header::COOKIE,
                ]))
                .layer(TraceLayer::new_for_http())
                .layer(TimeoutLayer::with_status_code(
                    axum::http::StatusCode::REQUEST_TIMEOUT,
                    state.cfg.request_timeout,
                ))
                .layer(cors)
                .layer(CompressionLayer::new()),
        )
        .with_state(state)
}

/// Pagination query (`page` is 1-based; `page_size` is capped at 100).
#[derive(Debug, serde::Deserialize)]
pub struct Page {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

impl Page {
    pub fn limit_offset(&self) -> (i64, i64, i64) {
        let size = self.page_size.unwrap_or(20).clamp(1, 100);
        let page = self.page.unwrap_or(1).max(1);
        (size, (page - 1) * size, page)
    }
}
