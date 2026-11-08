use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use super::{ClientIp, Page};
use crate::auth::{AuthUser, Role};
use crate::error::ApiResult;
use crate::state::AppState;

pub async fn status(State(state): State<AppState>, _u: AuthUser) -> ApiResult<impl IntoResponse> {
    let (head_seq, head_hash) = state.db.ledger_head().await?;
    let stats = state.db.stats(None).await?;
    let chain = match &state.ledger.chain {
        Some(c) => {
            let live = c.status().await;
            json!({
                "enabled": true, "contract": format!("{:#x}", c.address), "chain_id": c.chain_id,
                "signer": format!("{:#x}", c.signer_address()),
                "reachable": live.is_ok(), "block_number": live.as_ref().ok().map(|s| s.0),
                "signer_is_writer": live.as_ref().ok().map(|s| s.1), "error": live.err(),
                "explorer": c.explorer_url,
            })
        }
        None => json!({"enabled": false}),
    };
    let ipfs = match &state.ledger.ipfs {
        Some(i) => json!({"enabled": true, "reachable": i.healthy().await, "gateway": i.gateway}),
        None => json!({"enabled": false}),
    };
    Ok(Json(json!({
        "head": {"seq": head_seq, "hash": head_hash},
        "entries_by_status": stats.ledger_by_status,
        "chain": chain, "ipfs": ipfs,
        "mode": if state.ledger.chain.is_some() { "blockchain" } else if state.ledger.ipfs.is_some() { "ipfs" } else { "local-hash-chain" },
    })))
}

pub async fn entries(
    State(state): State<AppState>,
    user: AuthUser,
    Query(p): Query<Page>,
) -> ApiResult<impl IntoResponse> {
    user.require(Role::Reviewer)?;
    let (limit, offset, page) = p.limit_offset();
    let (rows, total) = state.db.ledger_page(limit, offset).await?;
    Ok(Json(
        json!({"items": rows, "total": total, "page": page, "page_size": limit}),
    ))
}

pub async fn verify(State(state): State<AppState>, user: AuthUser) -> ApiResult<impl IntoResponse> {
    user.require(Role::Reviewer)?;
    Ok(Json(state.ledger.verify_chain().await?))
}

pub async fn requeue(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
) -> ApiResult<impl IntoResponse> {
    user.require(Role::Admin)?;
    let n = state.db.ledger_requeue_failed().await?;
    state
        .db
        .audit(
            Some(&user.id),
            "ledger.requeue",
            None,
            Some(&ip),
            Some(format!("{n} entries")),
        )
        .await;
    Ok(Json(json!({"requeued": n})))
}
