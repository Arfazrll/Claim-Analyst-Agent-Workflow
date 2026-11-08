//! API errors. Every failure becomes a small JSON body; internal details are logged,
//! never returned (no stack traces or SQL in responses).

use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use claim_engine::FieldError;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("validation failed")]
    Validation(Vec<FieldError>),
    #[error("{0}")]
    BadRequest(String),
    #[error("authentication required")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error("too many requests")]
    RateLimited { retry_after_secs: u64 },
    #[error("account temporarily locked")]
    Locked { retry_after_secs: u64 },
    #[error("service unavailable: {0}")]
    Unavailable(String),
    #[error(transparent)]
    Internal(#[from] anyhow_like::Error),
}

/// Minimal boxed error so we do not need `anyhow` as a dependency.
pub mod anyhow_like {
    #[derive(Debug)]
    pub struct Error(pub String);
    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }
    impl std::error::Error for Error {}
}

impl ApiError {
    pub fn internal(e: impl std::fmt::Display) -> Self {
        ApiError::Internal(anyhow_like::Error(e.to_string()))
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        match &e {
            sqlx::Error::RowNotFound => ApiError::NotFound,
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                ApiError::Conflict("resource already exists".into())
            }
            _ => ApiError::internal(e),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message, extra) = match &self {
            ApiError::Validation(errors) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_error",
                "The request body is invalid.".to_string(),
                json!({"errors": errors}),
            ),
            ApiError::BadRequest(m) => {
                (StatusCode::BAD_REQUEST, "bad_request", m.clone(), json!({}))
            }
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Sign in to continue.".to_string(),
                json!({}),
            ),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, "forbidden", m.clone(), json!({})),
            ApiError::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "The requested resource does not exist.".to_string(),
                json!({}),
            ),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m.clone(), json!({})),
            ApiError::RateLimited { retry_after_secs } => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                format!("Too many requests. Try again in {retry_after_secs} s."),
                json!({"retry_after_secs": retry_after_secs}),
            ),
            ApiError::Locked { retry_after_secs } => (
                StatusCode::LOCKED,
                "account_locked",
                "Too many failed sign-in attempts. Try again later.".to_string(),
                json!({"retry_after_secs": retry_after_secs}),
            ),
            ApiError::Unavailable(m) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                m.clone(),
                json!({}),
            ),
            ApiError::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "The request could not be processed.".to_string(),
                    json!({}),
                )
            }
        };
        let mut body = json!({"error": code, "message": message});
        if let (Some(b), Some(x)) = (body.as_object_mut(), extra.as_object()) {
            b.extend(x.clone());
        }
        let mut resp = (status, Json(body)).into_response();
        if let ApiError::RateLimited { retry_after_secs } | ApiError::Locked { retry_after_secs } =
            self
        {
            if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                resp.headers_mut().insert("retry-after", v);
            }
        }
        resp
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
