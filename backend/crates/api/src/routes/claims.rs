use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use claim_engine::{AnalysisResponse, Decision};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ClientIp, Page};
use crate::auth::{AuthUser, Role};
use crate::crypto::{sha256_hex, sha256_json};
use crate::db::{now_ms, ClaimFilter, ClaimRow, LedgerEntry, NewClaim};
use crate::error::{ApiError, ApiResult};
use crate::ledger::EntryKind;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateIn {
    case: Value,
    llm_interpretation: Option<bool>,
}

#[derive(Deserialize)]
pub struct InterpretQuery {
    llm_interpretation: Option<bool>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    decision: Option<String>,
    status: Option<String>,
    q: Option<String>,
    // Not `#[serde(flatten)]`: flattening breaks numeric parsing in query strings.
    page: Option<i64>,
    page_size: Option<i64>,
}

#[derive(Deserialize)]
pub struct ReviewIn {
    action: String,
    final_decision: Option<String>,
    #[serde(default)]
    notes: String,
}

#[derive(Deserialize, Default)]
pub struct ReanalyzeIn {
    llm_interpretation: Option<bool>,
}

fn aad_case(id: &str) -> String {
    format!("claim:{id}:case")
}
fn aad_analysis(id: &str) -> String {
    format!("claim:{id}:analysis")
}
fn aad_notes(id: &str) -> String {
    format!("review:{id}:notes")
}

/// Run the engine, memoised in the shared cache. Analysis is deterministic for a given
/// engine version, case and LLM setting, so identical submissions skip recomputation.
async fn analyze_cached(
    state: &AppState,
    case: Value,
    interpret: Option<bool>,
) -> ApiResult<(AnalysisResponse, bool)> {
    claim_engine::validate_case(&case).map_err(ApiError::Validation)?;
    let interpret = interpret.unwrap_or(state.cfg.llm_interpretation_default);
    let key = format!(
        "analysis:{}",
        sha256_hex(
            format!(
                "{}|{}|{}|{}",
                claim_engine::ENGINE_VERSION,
                state.llm.provider(),
                interpret,
                crate::crypto::canonical_json(&case)
            )
            .as_bytes()
        )
    );
    if let Some(hit) = state
        .cache
        .get(&key)
        .await
        .and_then(|s| serde_json::from_str::<AnalysisResponse>(&s).ok())
    {
        state
            .metrics
            .analysis_cache_hits
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return Ok((hit, true));
    }
    let st = state
        .engine
        .analyze_unchecked(case, state.llm.as_ref(), Some(interpret))
        .await;
    let resp = st.to_response();
    state
        .metrics
        .analyses
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    state.metrics.decision(resp.decision.as_str());
    if let Ok(s) = serde_json::to_string(&resp) {
        state.cache.set(&key, &s, state.cfg.cache_ttl).await;
    }
    Ok((resp, false))
}

fn cache_header(mut resp: Response, hit: bool) -> Response {
    resp.headers_mut().insert(
        "x-cache",
        HeaderValue::from_static(if hit { "HIT" } else { "MISS" }),
    );
    resp
}

fn summary(c: &ClaimRow) -> Value {
    json!({
        "id": c.id, "case_id": c.case_id, "policy_id": c.policy_id, "status": c.status, "decision": c.decision,
        "final_decision": c.final_decision, "confidence": c.confidence, "claimed_total": c.claimed_total,
        "deductions": c.deductions, "payable": c.payable, "engine_version": c.engine_version, "version": c.version,
        "created_by": c.created_by, "created_by_name": c.created_by_name, "created_at": c.created_at, "updated_at": c.updated_at,
        "payload_hash": c.payload_hash, "analysis_hash": c.analysis_hash,
    })
}

fn ledger_json(state: &AppState, e: &LedgerEntry) -> Value {
    let tx_url = match (&state.ledger.chain, &e.tx_hash) {
        (Some(c), Some(tx)) => c.tx_url(tx),
        _ => None,
    };
    let ipfs_url = match (&state.ledger.ipfs, &e.cid) {
        (Some(i), Some(cid)) => Some(i.gateway_url(cid)),
        _ => None,
    };
    let mut v = serde_json::to_value(e).unwrap_or_default();
    v["tx_url"] = json!(tx_url);
    v["ipfs_url"] = json!(ipfs_url);
    v["record_id"] = json!(crate::ledger::record_id(&e.claim_id));
    v
}

fn ledger_doc(
    claim_id: &str,
    case_id: &str,
    version: i64,
    case_hash: &str,
    analysis_hash: &str,
    a: &AnalysisResponse,
) -> Value {
    json!({
        "claim_id": claim_id, "case_id": case_id, "version": version, "case_hash": case_hash, "analysis_hash": analysis_hash,
        "decision": a.decision, "confidence": a.confidence, "estimated_payable_inr": a.amounts.estimated_payable_inr,
        "total_deductions_inr": a.amounts.total_deductions_inr, "engine_version": a.engine_version,
        "validation": a.validation.status,
    })
}

async fn load_claim(state: &AppState, user: &AuthUser, id: &str) -> ApiResult<ClaimRow> {
    if id.len() > 64 {
        return Err(ApiError::NotFound);
    }
    let claim = state.db.claim(id).await?.ok_or(ApiError::NotFound)?;
    // Analysts only see their own claims; answer 404 (not 403) to avoid id probing.
    if user.role == Role::Analyst && claim.created_by != user.id {
        return Err(ApiError::NotFound);
    }
    Ok(claim)
}

async fn invalidate_stats(state: &AppState, user: &AuthUser) {
    state.cache.delete("stats:all").await;
    state.cache.delete(&format!("stats:{}", user.id)).await;
}

/// `POST /api/analyze` - stateless decision contract (compatible with the reference API).
pub async fn analyze_stateless(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(q): Query<InterpretQuery>,
    Json(case): Json<Value>,
) -> ApiResult<Response> {
    let (resp, hit) = analyze_cached(&state, case, q.llm_interpretation).await?;
    Ok(cache_header(Json(resp).into_response(), hit))
}

/// `POST /api/claims` - analyse, encrypt, persist and append to the ledger.
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    Json(body): Json<CreateIn>,
) -> ApiResult<Response> {
    let idem = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let idem_key = match &idem {
        Some(k) if k.len() > 128 || k.is_empty() => {
            return Err(ApiError::BadRequest(
                "Idempotency-Key must be 1-128 characters.".into(),
            ))
        }
        Some(k) => Some(format!("idem:{}:{}", user.id, sha256_hex(k.as_bytes()))),
        None => None,
    };
    if let Some(k) = &idem_key {
        if !state
            .cache
            .set_nx(k, "pending", Duration::from_secs(24 * 3600))
            .await
        {
            return match state.cache.get(k).await.as_deref() {
                Some("pending") | None => Err(ApiError::Conflict(
                    "A request with this Idempotency-Key is still being processed.".into(),
                )),
                Some(existing) => {
                    let claim = load_claim(&state, &user, existing).await?;
                    let mut r = Json(json!({"claim": summary(&claim), "idempotent_replay": true}))
                        .into_response();
                    r.headers_mut()
                        .insert("idempotent-replay", HeaderValue::from_static("true"));
                    Ok(r)
                }
            };
        }
    }

    let result: ApiResult<(Response, String)> = async {
        let case = body.case;
        let (analysis, hit) = analyze_cached(&state, case.clone(), body.llm_interpretation).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let analysis_v = serde_json::to_value(&analysis).map_err(ApiError::internal)?;
        let case_hash = sha256_json(&case);
        let analysis_hash = sha256_json(&analysis_v);
        let case_id = crate::sanitize(case.get("case_id").and_then(Value::as_str).unwrap_or("UNKNOWN"), 128);
        let policy_id = case.get("policy_id").and_then(Value::as_str).map(|s| crate::sanitize(s, 128));
        let payload_enc = state.cipher.encrypt_json(&case, &aad_case(&id));
        let analysis_enc = state.cipher.encrypt_json(&analysis_v, &aad_analysis(&id));
        let decision = analysis.decision.as_str();
        let new = NewClaim {
            id: &id,
            case_id: &case_id,
            policy_id: policy_id.as_deref(),
            decision,
            confidence: analysis.confidence,
            claimed_total: analysis.amounts.claimed_total_inr,
            deductions: analysis.amounts.total_deductions_inr,
            payable: analysis.amounts.estimated_payable_inr,
            payload_enc: &payload_enc,
            analysis_enc: &analysis_enc,
            payload_hash: &case_hash,
            analysis_hash: &analysis_hash,
            engine_version: &analysis.engine_version,
            created_by: &user.id,
        };
        let mut tx = state.db.pool.begin().await?;
        state.db.insert_claim(&mut tx, &new).await?;
        let entry = state
            .ledger
            .append(&mut tx, &id, EntryKind::ClaimAnalyzed, ledger_doc(&id, &case_id, 1, &case_hash, &analysis_hash, &analysis), &user.id)
            .await?;
        tx.commit().await?;

        state.db.audit(Some(&user.id), "claim.create", Some(&id), Some(&ip), Some(format!("decision={decision}"))).await;
        invalidate_stats(&state, &user).await;
        let claim = state.db.claim(&id).await?.ok_or(ApiError::NotFound)?;
        let resp = (StatusCode::CREATED, Json(json!({"claim": summary(&claim), "analysis": analysis, "ledger_entry": ledger_json(&state, &entry)}))).into_response();
        Ok((cache_header(resp, hit), id))
    }
    .await;

    match (result, &idem_key) {
        // Remember which claim this key created so a retry replays it.
        (Ok((resp, id)), Some(k)) => {
            state
                .cache
                .set(k, &id, Duration::from_secs(24 * 3600))
                .await;
            Ok(resp)
        }
        (Ok((resp, _)), None) => Ok(resp),
        (Err(e), Some(k)) => {
            state.cache.delete(k).await;
            Err(e)
        }
        (Err(e), None) => Err(e),
    }
}

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<impl IntoResponse> {
    let (limit, offset, page) = Page {
        page: q.page,
        page_size: q.page_size,
    }
    .limit_offset();
    let decision = q.decision.filter(|d| Decision::parse(d).is_some());
    let status = q
        .status
        .filter(|s| matches!(s.as_str(), "analyzed" | "reviewed" | "escalated"));
    let filter = ClaimFilter {
        decision,
        status,
        search: q
            .q
            .map(|s| s.chars().take(64).collect())
            .filter(|s: &String| !s.trim().is_empty()),
        owner: (user.role == Role::Analyst).then(|| user.id.clone()),
        limit,
        offset,
    };
    let (rows, total) = state.db.list_claims(&filter).await?;
    Ok(Json(
        json!({"items": rows.iter().map(summary).collect::<Vec<_>>(), "total": total, "page": page, "page_size": limit}),
    ))
}

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let claim = load_claim(&state, &user, &id).await?;
    let case = state
        .cipher
        .decrypt_json(&claim.payload_enc, &aad_case(&claim.id))
        .map_err(ApiError::internal)?;
    let analysis = state
        .cipher
        .decrypt_json(&claim.analysis_enc, &aad_analysis(&claim.id))
        .map_err(ApiError::internal)?;
    let reviews: Vec<Value> = state
        .db
        .reviews(&claim.id)
        .await?
        .into_iter()
        .map(|r| {
            let notes = state
                .cipher
                .decrypt(&r.notes_enc, aad_notes(&r.id).as_bytes())
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_default();
            let mut v = serde_json::to_value(&r).unwrap_or_default();
            v["notes"] = json!(notes);
            v
        })
        .collect();
    let ledger: Vec<Value> = state
        .db
        .ledger_for_claim(&claim.id)
        .await?
        .iter()
        .map(|e| ledger_json(&state, e))
        .collect();
    Ok(Json(
        json!({"claim": summary(&claim), "case": case, "analysis": analysis, "reviews": reviews, "ledger": ledger}),
    ))
}

pub async fn review(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    Path(id): Path<String>,
    Json(body): Json<ReviewIn>,
) -> ApiResult<impl IntoResponse> {
    user.require(Role::Reviewer)?;
    let claim = load_claim(&state, &user, &id).await?;
    // Four-eyes principle: nobody signs off their own submission (admins excepted).
    if claim.created_by == user.id && user.role != Role::Admin {
        return Err(ApiError::Forbidden(
            "You cannot review a claim you submitted.".into(),
        ));
    }
    let notes = crate::sanitize_multiline(&body.notes, 2000);
    let (status, final_decision) = match body.action.as_str() {
        "agree" => ("reviewed", claim.decision.clone()),
        "override" => {
            let d = body
                .final_decision
                .as_deref()
                .and_then(Decision::parse)
                .ok_or_else(|| {
                    ApiError::BadRequest("Choose the decision that replaces the engine's.".into())
                })?;
            if d.as_str() == claim.decision {
                return Err(ApiError::BadRequest(
                    "An override must change the decision; use agree instead.".into(),
                ));
            }
            if notes.trim().len() < 10 {
                return Err(ApiError::BadRequest(
                    "Explain the override in the notes (at least 10 characters).".into(),
                ));
            }
            ("reviewed", d.as_str().to_string())
        }
        "escalate" => {
            if notes.trim().len() < 10 {
                return Err(ApiError::BadRequest(
                    "Say why the claim is escalated (at least 10 characters).".into(),
                ));
            }
            ("escalated", Decision::NeedsReview.as_str().to_string())
        }
        _ => {
            return Err(ApiError::BadRequest(
                "action must be agree, override or escalate".into(),
            ))
        }
    };
    let review_id = uuid::Uuid::new_v4().to_string();
    let notes_enc = state
        .cipher
        .encrypt(notes.as_bytes(), aad_notes(&review_id).as_bytes());
    let doc = json!({
        "claim_id": claim.id, "review_id": review_id, "action": body.action, "engine_decision": claim.decision,
        "final_decision": final_decision, "reviewer_id": user.id, "notes_hash": sha256_hex(notes.as_bytes()),
        "analysis_hash": claim.analysis_hash, "reviewed_at": now_ms(),
    });
    let mut tx = state.db.pool.begin().await?;
    state
        .db
        .insert_review(
            &mut tx,
            &review_id,
            &claim.id,
            &user.id,
            &body.action,
            &final_decision,
            &notes_enc,
        )
        .await?;
    state
        .db
        .set_claim_review(&mut tx, &claim.id, status, &final_decision)
        .await?;
    let entry = state
        .ledger
        .append(&mut tx, &claim.id, EntryKind::ClaimReviewed, doc, &user.id)
        .await?;
    tx.commit().await?;
    state
        .db
        .audit(
            Some(&user.id),
            "claim.review",
            Some(&claim.id),
            Some(&ip),
            Some(format!("action={} final={final_decision}", body.action)),
        )
        .await;
    invalidate_stats(&state, &user).await;
    let claim = state.db.claim(&claim.id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(
        json!({"claim": summary(&claim), "ledger_entry": ledger_json(&state, &entry)}),
    ))
}

pub async fn reanalyze(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    Path(id): Path<String>,
    body: Option<Json<ReanalyzeIn>>,
) -> ApiResult<impl IntoResponse> {
    let claim = load_claim(&state, &user, &id).await?;
    let interpret = body.and_then(|b| b.0.llm_interpretation);
    let case = state
        .cipher
        .decrypt_json(&claim.payload_enc, &aad_case(&claim.id))
        .map_err(ApiError::internal)?;
    let (analysis, _) = analyze_cached(&state, case, interpret).await?;
    let analysis_v = serde_json::to_value(&analysis).map_err(ApiError::internal)?;
    let analysis_hash = sha256_json(&analysis_v);
    let analysis_enc = state
        .cipher
        .encrypt_json(&analysis_v, &aad_analysis(&claim.id));
    let upd = NewClaim {
        id: &claim.id,
        case_id: &claim.case_id,
        policy_id: claim.policy_id.as_deref(),
        decision: analysis.decision.as_str(),
        confidence: analysis.confidence,
        claimed_total: analysis.amounts.claimed_total_inr,
        deductions: analysis.amounts.total_deductions_inr,
        payable: analysis.amounts.estimated_payable_inr,
        payload_enc: &claim.payload_enc,
        analysis_enc: &analysis_enc,
        payload_hash: &claim.payload_hash,
        analysis_hash: &analysis_hash,
        engine_version: &analysis.engine_version,
        created_by: &claim.created_by,
    };
    let mut tx = state.db.pool.begin().await?;
    let version = state.db.update_claim_analysis(&mut tx, &upd).await?;
    let doc = ledger_doc(
        &claim.id,
        &claim.case_id,
        version,
        &claim.payload_hash,
        &analysis_hash,
        &analysis,
    );
    let entry = state
        .ledger
        .append(
            &mut tx,
            &claim.id,
            EntryKind::ClaimReanalyzed,
            doc,
            &user.id,
        )
        .await?;
    tx.commit().await?;
    state
        .db
        .audit(
            Some(&user.id),
            "claim.reanalyze",
            Some(&claim.id),
            Some(&ip),
            Some(format!("version={version}")),
        )
        .await;
    invalidate_stats(&state, &user).await;
    let claim = state.db.claim(&claim.id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(
        json!({"claim": summary(&claim), "analysis": analysis, "ledger_entry": ledger_json(&state, &entry)}),
    ))
}

pub async fn proof(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let claim = load_claim(&state, &user, &id).await?;
    let case = state
        .cipher
        .decrypt_json(&claim.payload_enc, &aad_case(&claim.id))
        .ok();
    let analysis = state
        .cipher
        .decrypt_json(&claim.analysis_enc, &aad_analysis(&claim.id))
        .ok();
    let checks = state
        .ledger
        .prove_claim(&claim, case.as_ref(), analysis.as_ref())
        .await?;
    let ok = checks.iter().all(|c| c.passed);
    let entries: Vec<Value> = state
        .db
        .ledger_for_claim(&claim.id)
        .await?
        .iter()
        .map(|e| ledger_json(&state, e))
        .collect();
    Ok(Json(json!({
        "claim_id": claim.id, "ok": ok, "checks": checks, "entries": entries,
        "record_id": crate::ledger::record_id(&claim.id),
        "chain": state.ledger.chain.as_ref().map(|c| json!({"contract": format!("{:#x}", c.address), "chain_id": c.chain_id})),
        "verified_at": now_ms(),
    })))
}

pub async fn export(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let claim = load_claim(&state, &user, &id).await?;
    let case = state
        .cipher
        .decrypt_json(&claim.payload_enc, &aad_case(&claim.id))
        .map_err(ApiError::internal)?;
    let analysis = state
        .cipher
        .decrypt_json(&claim.analysis_enc, &aad_analysis(&claim.id))
        .map_err(ApiError::internal)?;
    let checks = state
        .ledger
        .prove_claim(&claim, Some(&case), Some(&analysis))
        .await?;
    let ledger: Vec<Value> = state
        .db
        .ledger_for_claim(&claim.id)
        .await?
        .iter()
        .map(|e| ledger_json(&state, e))
        .collect();
    let mut bundle = json!({
        "format": "caw-claim-export/v1", "exported_at": now_ms(), "exported_by": user.email,
        "claim": summary(&claim), "case": case, "analysis": analysis, "ledger": ledger, "integrity_checks": checks,
    });
    bundle["export_sha256"] = json!(sha256_json(&bundle));
    state
        .db
        .audit(
            Some(&user.id),
            "claim.export",
            Some(&claim.id),
            Some(&ip),
            None,
        )
        .await;
    let mut resp = Json(bundle).into_response();
    let name = format!(
        "attachment; filename=\"claim-{}.json\"",
        claim
            .case_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>()
    );
    if let Ok(v) = HeaderValue::from_str(&name) {
        resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(resp)
}

pub async fn samples(_user: AuthUser) -> impl IntoResponse {
    let items: Vec<Value> = claim_engine::eval::bundled_cases()
        .into_iter()
        .map(|c| {
            let expected = c.get("case_id").and_then(Value::as_str).and_then(claim_engine::eval::expected).map(|e| e.decision);
            json!({"case_id": c["case_id"], "task": c.get("task"), "diagnosis": c.pointer("/treatment/diagnosis"), "expected_decision": expected, "case": c})
        })
        .collect();
    (
        [(header::CACHE_CONTROL, "private, max-age=3600")],
        Json(json!({"items": items})),
    )
}

pub async fn stats(State(state): State<AppState>, user: AuthUser) -> ApiResult<impl IntoResponse> {
    let scope_all = user.role >= Role::Reviewer;
    let key = if scope_all {
        "stats:all".to_string()
    } else {
        format!("stats:{}", user.id)
    };
    if let Some(hit) = state
        .cache
        .get(&key)
        .await
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    {
        return Ok(Json(hit));
    }
    let s = state
        .db
        .stats(if scope_all { None } else { Some(&user.id) })
        .await?;
    let v = json!({"scope": if scope_all { "all" } else { "mine" }, "stats": s, "generated_at": now_ms()});
    state
        .cache
        .set(&key, &v.to_string(), Duration::from_secs(15))
        .await;
    Ok(Json(v))
}
