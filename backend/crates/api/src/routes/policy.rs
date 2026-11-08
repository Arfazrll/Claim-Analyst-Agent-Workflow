use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use claim_engine::dimensions::DIMENSIONS;
use claim_engine::facts::Facts;
use claim_engine::TopK;
use serde::Deserialize;
use serde_json::json;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
    k: Option<usize>,
}

fn etag() -> String {
    format!(
        "\"policy-{}\"",
        &crate::crypto::sha256_hex(claim_engine::BUNDLED_POLICY_JSONL.as_bytes())[..16]
    )
}

/// The policy text is static per release: cacheable by the browser, revalidated by ETag.
fn cached(headers: &HeaderMap, body: serde_json::Value) -> Response {
    let tag = etag();
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(tag.as_str())
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    let mut r = Json(body).into_response();
    let h = r.headers_mut();
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    if let Ok(v) = HeaderValue::from_str(&tag) {
        h.insert(header::ETAG, v);
    }
    r
}

pub async fn chunks(State(state): State<AppState>, _u: AuthUser, headers: HeaderMap) -> Response {
    let items: Vec<_> = state.engine.retriever().chunks().iter().map(|c| json!({
        "chunk_id": c.chunk_id, "section": c.section, "label": c.label, "page_start": c.page_start, "page_end": c.page_end, "text": c.text,
    })).collect();
    cached(
        &headers,
        json!({"source": "USGIC CSC Individual Health Insurance (2017-2018) policy wording", "items": items}),
    )
}

pub async fn chunk(
    State(state): State<AppState>,
    _u: AuthUser,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let c = state
        .engine
        .retriever()
        .chunk(&id)
        .ok_or(ApiError::NotFound)?;
    Ok(cached(
        &headers,
        serde_json::to_value(c).map_err(ApiError::internal)?,
    ))
}

/// Hybrid search with every stage exposed (vector, BM25, fused, reranked).
pub async fn search(
    State(state): State<AppState>,
    _u: AuthUser,
    Query(q): Query<SearchQuery>,
) -> ApiResult<impl IntoResponse> {
    let query = q.q.trim();
    if query.is_empty() || query.len() > 300 {
        return Err(ApiError::BadRequest(
            "Enter a search of 1-300 characters.".into(),
        ));
    }
    let k = TopK {
        final_k: q.k.unwrap_or(5).clamp(1, 10),
        ..TopK::default()
    };
    let key = format!(
        "search:{}:{}",
        k.final_k,
        crate::crypto::sha256_hex(query.to_lowercase().as_bytes())
    );
    if let Some(hit) = state
        .cache
        .get(&key)
        .await
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    {
        return Ok(Json(hit));
    }
    let retriever = state.engine.retriever();
    let stages = retriever.search_stages(query, k);
    let results = retriever.search(query, k);
    let v = json!({"query": query, "results": results, "stages": stages});
    state
        .cache
        .set(&key, &v.to_string(), state.cfg.cache_ttl)
        .await;
    Ok(Json(v))
}

pub async fn dimensions(_u: AuthUser) -> impl IntoResponse {
    let f = Facts::default();
    let items: Vec<_> = DIMENSIONS
        .iter()
        .map(|d| json!({"key": d.key, "title": d.title, "query": (d.query)(&f)}))
        .collect();
    (
        [(header::CACHE_CONTROL, "private, max-age=3600")],
        Json(json!({"items": items, "synonyms": claim_engine::dimensions::CLINICAL_SYNONYMS})),
    )
}
