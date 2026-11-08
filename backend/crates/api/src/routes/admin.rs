use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::{ClientIp, Page};
use crate::auth::{AuthUser, Role};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct UpdateUser {
    role: Option<String>,
    active: Option<bool>,
}

pub async fn users(State(state): State<AppState>, user: AuthUser) -> ApiResult<impl IntoResponse> {
    user.require(Role::Admin)?;
    Ok(Json(json!({"items": state.db.list_users().await?})))
}

pub async fn update_user(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    Path(id): Path<String>,
    Json(body): Json<UpdateUser>,
) -> ApiResult<impl IntoResponse> {
    user.require(Role::Admin)?;
    let target = state.db.user_by_id(&id).await?.ok_or(ApiError::NotFound)?;
    let role = match body.role.as_deref() {
        Some(r) => Role::parse(r).ok_or_else(|| {
            ApiError::BadRequest("role must be analyst, reviewer or admin".into())
        })?,
        None => Role::parse(&target.role).unwrap_or(Role::Analyst),
    };
    let active = body.active.unwrap_or(target.active);
    if target.id == user.id && (role != Role::Admin || !active) {
        return Err(ApiError::BadRequest(
            "You cannot remove your own admin access.".into(),
        ));
    }
    state
        .db
        .update_user_admin(&id, role.as_str(), active)
        .await?;
    if !active {
        state.db.revoke_user_tokens(&id).await?;
    }
    state.cache.delete(&format!("user:{id}")).await;
    state
        .db
        .audit(
            Some(&user.id),
            "admin.update_user",
            Some(&id),
            Some(&ip),
            Some(format!("role={} active={active}", role.as_str())),
        )
        .await;
    let updated = state.db.user_by_id(&id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(json!({"user": updated})))
}

pub async fn audit(
    State(state): State<AppState>,
    user: AuthUser,
    Query(p): Query<Page>,
) -> ApiResult<impl IntoResponse> {
    user.require(Role::Admin)?;
    let (limit, offset, page) = p.limit_offset();
    let (rows, total) = state.db.audit_page(limit, offset).await?;
    Ok(Json(
        json!({"items": rows, "total": total, "page": page, "page_size": limit}),
    ))
}

/// Run the bundled evaluation suite (deterministic, offline). Cached per engine version.
pub async fn eval(State(state): State<AppState>, user: AuthUser) -> ApiResult<impl IntoResponse> {
    user.require(Role::Reviewer)?;
    let key = format!("eval:{}", claim_engine::ENGINE_VERSION);
    if let Some(hit) = state
        .cache
        .get(&key)
        .await
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    {
        return Ok(Json(hit));
    }
    let report =
        claim_engine::eval::run_suite(&state.engine, &claim_engine::eval::bundled_cases()).await;
    let v = serde_json::to_value(&report).map_err(ApiError::internal)?;
    state
        .cache
        .set(&key, &v.to_string(), state.cfg.cache_ttl)
        .await;
    Ok(Json(v))
}
