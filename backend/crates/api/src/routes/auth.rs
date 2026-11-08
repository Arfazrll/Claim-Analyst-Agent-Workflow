use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use serde_json::json;

use super::ClientIp;
use crate::auth::{
    clear_cookies, hash_password, issue_access, new_csrf, password_problems, session_cookies,
    valid_email, verify_password, AuthUser, Role, REFRESH_COOKIE,
};
use crate::crypto::{random_token, sha256_hex};
use crate::db::{now_ms, User};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_FAILED_LOGINS: i64 = 5;
const LOCK_MS: i64 = 15 * 60 * 1000;

#[derive(Deserialize)]
pub struct RegisterIn {
    email: String,
    name: String,
    password: String,
}

#[derive(Deserialize)]
pub struct LoginIn {
    email: String,
    password: String,
}

#[derive(Deserialize)]
pub struct PasswordIn {
    current_password: String,
    new_password: String,
}

fn user_json(u: &User) -> serde_json::Value {
    json!({"id": u.id, "email": u.email, "name": u.name, "role": u.role, "active": u.active, "created_at": u.created_at})
}

fn user_agent(h: &HeaderMap) -> Option<String> {
    h.get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Issue access + refresh + CSRF cookies for `user` (refresh token stored hashed).
pub async fn start_session(
    state: &AppState,
    jar: CookieJar,
    user: &User,
    family: Option<String>,
    ua: Option<String>,
    ip: &str,
) -> ApiResult<CookieJar> {
    let role = Role::parse(&user.role).unwrap_or(Role::Analyst);
    let csrf = new_csrf();
    let access = issue_access(&state.cfg, &user.id, &user.email, &user.name, role, &csrf)?;
    let refresh = random_token(32);
    let family = family.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let expires = now_ms() + state.cfg.refresh_ttl.as_millis() as i64;
    let id = uuid::Uuid::new_v4().to_string();
    state
        .db
        .insert_refresh(
            &id,
            &user.id,
            &sha256_hex(refresh.as_bytes()),
            &family,
            expires,
            ua.as_deref(),
            Some(ip),
        )
        .await?;
    Ok(session_cookies(&state.cfg, jar, access, refresh, csrf))
}

pub async fn register(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<RegisterIn>,
) -> ApiResult<impl IntoResponse> {
    if !state.cfg.allow_registration {
        return Err(ApiError::Forbidden(
            "Self-registration is disabled. Ask an administrator for an account.".into(),
        ));
    }
    let email = body.email.trim().to_lowercase();
    let name = crate::sanitize(&body.name, 80);
    if !valid_email(&email) {
        return Err(ApiError::BadRequest("Enter a valid email address.".into()));
    }
    if name.len() < 2 {
        return Err(ApiError::BadRequest(
            "Enter your name (at least 2 characters).".into(),
        ));
    }
    if let Some(p) = password_problems(&body.password, &email) {
        return Err(ApiError::BadRequest(p));
    }
    if state.db.user_by_email(&email).await?.is_some() {
        return Err(ApiError::Conflict(
            "An account with this email already exists.".into(),
        ));
    }
    let hash = hash_password(&body.password)?;
    let user = state
        .db
        .create_user(
            &uuid::Uuid::new_v4().to_string(),
            &email,
            &name,
            &hash,
            Role::Analyst.as_str(),
        )
        .await?;
    state
        .db
        .audit(
            Some(&user.id),
            "auth.register",
            Some(&user.id),
            Some(&ip),
            None,
        )
        .await;
    let jar = start_session(&state, jar, &user, None, user_agent(&headers), &ip).await?;
    Ok((
        StatusCode::CREATED,
        jar,
        Json(json!({"user": user_json(&user)})),
    ))
}

pub async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<LoginIn>,
) -> ApiResult<impl IntoResponse> {
    let email = body.email.trim().to_lowercase();
    let user = state.db.user_by_email(&email).await?;
    if let Some(u) = &user {
        let now = now_ms();
        if u.locked_until > now {
            return Err(ApiError::Locked {
                retry_after_secs: ((u.locked_until - now) / 1000).max(1) as u64,
            });
        }
    }
    let ok = verify_password(
        &body.password,
        user.as_ref().map(|u| u.password_hash.as_str()),
    );
    let Some(user) = user.filter(|u| ok && u.active) else {
        state
            .metrics
            .login_failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some(u) = state.db.user_by_email(&email).await? {
            state
                .db
                .record_failed_login(&u.id, MAX_FAILED_LOGINS, LOCK_MS)
                .await?;
        }
        state
            .db
            .audit(None, "auth.login_failed", Some(&email), Some(&ip), None)
            .await;
        return Err(ApiError::BadRequest("Invalid email or password.".into()));
    };
    state.db.reset_failed_logins(&user.id).await?;
    state
        .db
        .audit(
            Some(&user.id),
            "auth.login",
            Some(&user.id),
            Some(&ip),
            None,
        )
        .await;
    let jar = start_session(&state, jar, &user, None, user_agent(&headers), &ip).await?;
    Ok((jar, Json(json!({"user": user_json(&user)}))))
}

pub async fn refresh(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    jar: CookieJar,
) -> ApiResult<impl IntoResponse> {
    let Some(token) = jar
        .get(REFRESH_COOKIE)
        .map(|c| c.value().to_string())
        .filter(|t| !t.is_empty())
    else {
        return Err(ApiError::Unauthorized);
    };
    let Some(stored) = state
        .db
        .refresh_by_hash(&sha256_hex(token.as_bytes()))
        .await?
    else {
        return Ok((
            clear_cookies(&state.cfg, jar),
            Json(json!({"error": "unauthorized"})),
        )
            .into_response_with(StatusCode::UNAUTHORIZED));
    };
    if let Some(revoked) = stored.revoked_at {
        // Re-use of a rotated token means it was stolen (or two tabs raced). Outside a
        // short race window the whole session family is revoked.
        if now_ms() - revoked > 30_000 {
            state.db.revoke_family(&stored.family_id).await?;
            state
                .db
                .audit(
                    Some(&stored.user_id),
                    "auth.refresh_reuse_detected",
                    Some(&stored.family_id),
                    Some(&ip),
                    None,
                )
                .await;
            tracing::warn!(user = %stored.user_id, "refresh token reuse detected; session family revoked");
        }
        return Ok((
            clear_cookies(&state.cfg, jar),
            Json(json!({"error": "unauthorized"})),
        )
            .into_response_with(StatusCode::UNAUTHORIZED));
    }
    if stored.expires_at < now_ms() {
        return Ok((
            clear_cookies(&state.cfg, jar),
            Json(json!({"error": "unauthorized"})),
        )
            .into_response_with(StatusCode::UNAUTHORIZED));
    }
    let user = state
        .db
        .user_by_id(&stored.user_id)
        .await?
        .filter(|u| u.active)
        .ok_or(ApiError::Unauthorized)?;
    // Claim the old token atomically first: a concurrent refresh with the same token loses.
    if !state.db.rotate_refresh(&stored.id, "rotated").await? {
        return Err(ApiError::Unauthorized);
    }
    let new_jar = start_session(
        &state,
        jar,
        &user,
        Some(stored.family_id.clone()),
        user_agent(&headers),
        &ip,
    )
    .await?;
    Ok((new_jar, Json(json!({"user": user_json(&user)}))).into_response())
}

pub async fn logout(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    jar: CookieJar,
) -> ApiResult<impl IntoResponse> {
    if let Some(token) = jar.get(REFRESH_COOKIE).map(|c| c.value().to_string()) {
        if let Some(stored) = state
            .db
            .refresh_by_hash(&sha256_hex(token.as_bytes()))
            .await?
        {
            state.db.revoke_family(&stored.family_id).await?;
            state
                .db
                .audit(Some(&stored.user_id), "auth.logout", None, Some(&ip), None)
                .await;
        }
    }
    Ok((clear_cookies(&state.cfg, jar), StatusCode::NO_CONTENT))
}

pub async fn me(State(state): State<AppState>, user: AuthUser) -> ApiResult<impl IntoResponse> {
    let u = state
        .db
        .user_by_id(&user.id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    Ok(Json(json!({"user": user_json(&u)})))
}

pub async fn change_password(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    user: AuthUser,
    jar: CookieJar,
    Json(body): Json<PasswordIn>,
) -> ApiResult<impl IntoResponse> {
    let u = state
        .db
        .user_by_id(&user.id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    if !verify_password(&body.current_password, Some(&u.password_hash)) {
        return Err(ApiError::BadRequest(
            "The current password is incorrect.".into(),
        ));
    }
    if let Some(p) = password_problems(&body.new_password, &u.email) {
        return Err(ApiError::BadRequest(p));
    }
    state
        .db
        .update_password(&u.id, &hash_password(&body.new_password)?)
        .await?;
    state.db.revoke_user_tokens(&u.id).await?;
    state
        .db
        .audit(
            Some(&u.id),
            "auth.password_changed",
            Some(&u.id),
            Some(&ip),
            None,
        )
        .await;
    let jar = start_session(&state, jar, &u, None, user_agent(&headers), &ip).await?;
    Ok((jar, Json(json!({"ok": true}))))
}

trait WithStatus {
    fn into_response_with(self, status: StatusCode) -> axum::response::Response;
}

impl<T: IntoResponse> WithStatus for T {
    fn into_response_with(self, status: StatusCode) -> axum::response::Response {
        let mut r = self.into_response();
        *r.status_mut() = status;
        r
    }
}
