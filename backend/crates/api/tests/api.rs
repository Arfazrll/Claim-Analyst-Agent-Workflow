//! End-to-end API tests against an in-memory SQLite database and in-process cache.

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use claim_api::config::Config;
use claim_api::{router, AppState};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

const PASSWORD: &str = "Correct-Horse-Battery-9";

struct Client {
    app: Router,
    cookies: HashMap<String, String>,
}

struct Resp {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Value,
}

impl Client {
    fn new(app: Router) -> Self {
        Client {
            app,
            cookies: HashMap::new(),
        }
    }

    async fn send(
        &mut self,
        method: &str,
        path: &str,
        body: Option<Value>,
        csrf: bool,
        extra: &[(&str, &str)],
    ) -> Resp {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if !self.cookies.is_empty() {
            let c = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            req = req.header("cookie", c);
        }
        if csrf {
            if let Some(t) = self.cookies.get("caw_csrf") {
                req = req.header("x-csrf-token", t.clone());
            }
        }
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let req = req
            .body(
                body.map(|b| Body::from(b.to_string()))
                    .unwrap_or_else(Body::empty),
            )
            .unwrap();
        let resp = self.app.clone().oneshot(req).await.unwrap();
        for sc in resp.headers().get_all("set-cookie") {
            let s = sc.to_str().unwrap();
            let pair = s.split(';').next().unwrap();
            let (k, v) = pair.split_once('=').unwrap();
            if v.is_empty() {
                self.cookies.remove(k);
            } else {
                self.cookies.insert(k.to_string(), v.to_string());
            }
        }
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Resp {
            status,
            headers,
            body,
        }
    }

    async fn get(&mut self, path: &str) -> Resp {
        self.send("GET", path, None, false, &[]).await
    }
    async fn post(&mut self, path: &str, body: Value) -> Resp {
        self.send("POST", path, Some(body), true, &[]).await
    }
}

async fn setup(tweak: impl FnOnce(&mut Config)) -> (AppState, Router) {
    let mut cfg = Config::for_tests();
    cfg.admin_email = Some("admin@example.com".into());
    cfg.admin_password = Some("Bootstrap-Strong-Pass-7".into());
    tweak(&mut cfg);
    let state = AppState::build(cfg).await.expect("state");
    claim_api::bootstrap_admin(&state).await.unwrap();
    let app = router(state.clone());
    (state, app)
}

fn sample(id: &str) -> Value {
    claim_engine::eval::bundled_cases()
        .into_iter()
        .find(|c| c["case_id"] == id)
        .unwrap()
}

async fn register(app: &Router, email: &str) -> Client {
    let mut c = Client::new(app.clone());
    let r = c
        .send(
            "POST",
            "/api/auth/register",
            Some(json!({"email": email, "name": "Test User", "password": PASSWORD})),
            false,
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    c
}

async fn login(app: &Router, email: &str, password: &str) -> Client {
    let mut c = Client::new(app.clone());
    let r = c
        .send(
            "POST",
            "/api/auth/login",
            Some(json!({"email": email, "password": password})),
            false,
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    c
}

#[tokio::test]
async fn full_claim_lifecycle_with_ledger_proof() {
    let (state, app) = setup(|_| {}).await;
    let mut analyst = register(&app, "analyst@example.com").await;

    // Unsafe request without the CSRF header is refused.
    let r = analyst
        .send(
            "POST",
            "/api/claims",
            Some(json!({"case": sample("PUB-001")})),
            false,
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    let r = analyst
        .post("/api/claims", json!({"case": sample("PUB-001")}))
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(r.body["analysis"]["decision"], "ADMISSIBLE_WITH_LIMITS");
    assert_eq!(
        r.body["analysis"]["amounts"]["total_deductions_inr"],
        10200.0
    );
    assert_eq!(r.body["ledger_entry"]["seq"], 1);
    let id = r.body["claim"]["id"].as_str().unwrap().to_string();

    // Second identical submission is served from the analysis cache.
    let r2 = analyst
        .post("/api/claims", json!({"case": sample("PUB-001")}))
        .await;
    assert_eq!(r2.headers.get("x-cache").unwrap(), "HIT");

    let r = analyst.get(&format!("/api/claims/{id}")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["case"]["case_id"], "PUB-001");
    assert_eq!(r.body["ledger"].as_array().unwrap().len(), 1);

    // Analysts cannot review.
    let r = analyst
        .post(
            &format!("/api/claims/{id}/review"),
            json!({"action": "agree"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // Admin promotes a second user to reviewer; four-eyes review succeeds.
    let mut admin = login(&app, "admin@example.com", "Bootstrap-Strong-Pass-7").await;
    let _reviewer_client = register(&app, "reviewer@example.com").await;
    let users = admin.get("/api/admin/users").await;
    let rid = users.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["email"] == "reviewer@example.com")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = admin
        .send(
            "PATCH",
            &format!("/api/admin/users/{rid}"),
            Some(json!({"role": "reviewer"})),
            true,
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let mut reviewer = login(&app, "reviewer@example.com", PASSWORD).await;

    let r = reviewer
        .post(
            &format!("/api/claims/{id}/review"),
            json!({"action": "override", "final_decision": "NEEDS_REVIEW", "notes": "short"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "override needs a reason");
    let r = reviewer
        .post(
            &format!("/api/claims/{id}/review"),
            json!({"action": "agree", "notes": "Limits verified against clause 3."}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["claim"]["status"], "reviewed");

    // Integrity proof passes, whole chain verifies.
    let p = reviewer.get(&format!("/api/claims/{id}/proof")).await;
    assert_eq!(p.body["ok"], true, "{:?}", p.body["checks"]);
    let v = reviewer.get("/api/ledger/verify").await;
    assert_eq!(v.body["ok"], true, "{:?}", v.body);

    // Tamper with the stored analysis hash: the proof must fail.
    sqlx::query("UPDATE claims SET analysis_hash = 'deadbeef' WHERE id = $1")
        .bind(id.clone())
        .execute(&state.db.pool)
        .await
        .unwrap();
    let p = reviewer.get(&format!("/api/claims/{id}/proof")).await;
    assert_eq!(p.body["ok"], false);

    // Tamper with a ledger entry: chain verification must pinpoint it.
    sqlx::query("UPDATE ledger_entries SET payload_hash = 'x' WHERE seq = 1")
        .execute(&state.db.pool)
        .await
        .unwrap();
    let v = reviewer.get("/api/ledger/verify").await;
    assert_eq!(v.body["ok"], false);
    assert_eq!(v.body["first_broken_seq"], 1);
}

#[tokio::test]
async fn analysts_only_see_their_own_claims() {
    let (_s, app) = setup(|_| {}).await;
    let mut a = register(&app, "a1@example.com").await;
    let mut b = register(&app, "b1@example.com").await;
    let r = a
        .post("/api/claims", json!({"case": sample("PUB-002")}))
        .await;
    let id = r.body["claim"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        b.get(&format!("/api/claims/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(b.get("/api/claims").await.body["total"], 0);
    assert_eq!(a.get("/api/claims").await.body["total"], 1);
    // Pagination and filters in the query string (regression: numeric params).
    let r = a
        .get("/api/claims?page=1&page_size=5&status=analyzed&decision=NOT_ADMISSIBLE&q=pub")
        .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["total"], 1);
    assert_eq!(r.body["page_size"], 5);
    assert_eq!(
        a.get("/api/ledger/entries?page=1&page_size=5").await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn reviewers_cannot_review_their_own_submission() {
    let (_s, app) = setup(|_| {}).await;
    let mut admin = login(&app, "admin@example.com", "Bootstrap-Strong-Pass-7").await;
    let _ = register(&app, "rev@example.com").await;
    let users = admin.get("/api/admin/users").await;
    let rid = users.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["email"] == "rev@example.com")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    admin
        .send(
            "PATCH",
            &format!("/api/admin/users/{rid}"),
            Some(json!({"role": "reviewer"})),
            true,
            &[],
        )
        .await;
    let mut rev = login(&app, "rev@example.com", PASSWORD).await;
    let id = rev
        .post("/api/claims", json!({"case": sample("PUB-003")}))
        .await
        .body["claim"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = rev
        .post(
            &format!("/api/claims/{id}/review"),
            json!({"action": "agree"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn invalid_case_returns_field_errors() {
    let (_s, app) = setup(|_| {}).await;
    let mut a = register(&app, "v@example.com").await;
    let r = a
        .post("/api/claims", json!({"case": {"case_id": "X"}}))
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(r.body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["field"] == "treatment"));
}

#[tokio::test]
async fn idempotency_key_replays_the_same_claim() {
    let (_s, app) = setup(|_| {}).await;
    let mut a = register(&app, "idem@example.com").await;
    let body = json!({"case": sample("PUB-005")});
    let r1 = a
        .send(
            "POST",
            "/api/claims",
            Some(body.clone()),
            true,
            &[("idempotency-key", "abc-123")],
        )
        .await;
    let r2 = a
        .send(
            "POST",
            "/api/claims",
            Some(body),
            true,
            &[("idempotency-key", "abc-123")],
        )
        .await;
    assert_eq!(r1.status, StatusCode::CREATED);
    assert_eq!(r2.status, StatusCode::OK);
    assert_eq!(r1.body["claim"]["id"], r2.body["claim"]["id"]);
    assert_eq!(a.get("/api/claims").await.body["total"], 1);
}

#[tokio::test]
async fn login_lockout_and_generic_errors() {
    let (_s, app) = setup(|_| {}).await;
    let _ = register(&app, "lock@example.com").await;
    let mut c = Client::new(app.clone());
    for _ in 0..5 {
        let r = c
            .send(
                "POST",
                "/api/auth/login",
                Some(json!({"email": "lock@example.com", "password": "wrong-password"})),
                false,
                &[],
            )
            .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
        assert_eq!(r.body["message"], "Invalid email or password.");
    }
    let r = c
        .send(
            "POST",
            "/api/auth/login",
            Some(json!({"email": "lock@example.com", "password": PASSWORD})),
            false,
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::LOCKED);
    // Unknown accounts get the same message as wrong passwords.
    let r = c
        .send(
            "POST",
            "/api/auth/login",
            Some(json!({"email": "nobody@example.com", "password": "x"})),
            false,
            &[],
        )
        .await;
    assert_eq!(r.body["message"], "Invalid email or password.");
}

#[tokio::test]
async fn rate_limiter_returns_429_with_headers() {
    let (_s, app) = setup(|c| c.rate_auth_per_min = 3).await;
    let mut c = Client::new(app.clone());
    let mut last = StatusCode::OK;
    for _ in 0..6 {
        last = c
            .send(
                "POST",
                "/api/auth/login",
                Some(json!({"email": "x@example.com", "password": "y"})),
                false,
                &[],
            )
            .await
            .status;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
    let r = c
        .send(
            "POST",
            "/api/auth/login",
            Some(json!({"email": "x@example.com", "password": "y"})),
            false,
            &[],
        )
        .await;
    assert!(r.headers.contains_key("retry-after"));
    assert_eq!(r.headers.get("ratelimit-remaining").unwrap(), "0");
}

#[tokio::test]
async fn refresh_rotation_and_reuse_detection() {
    let (_s, app) = setup(|_| {}).await;
    let mut c = register(&app, "rot@example.com").await;
    let old_refresh = c.cookies.get("caw_refresh").cloned().unwrap();
    let r = c.send("POST", "/api/auth/refresh", None, false, &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_ne!(
        c.cookies.get("caw_refresh").unwrap(),
        &old_refresh,
        "refresh token rotates"
    );
    // Replaying the old token immediately is treated as a benign race (401, no revoke).
    let mut thief = Client::new(app.clone());
    thief.cookies.insert("caw_refresh".into(), old_refresh);
    assert_eq!(
        thief
            .send("POST", "/api/auth/refresh", None, false, &[])
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    // The legitimate session keeps working.
    assert_eq!(
        c.send("POST", "/api/auth/refresh", None, false, &[])
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn security_headers_cross_origin_and_auth_required() {
    let (_s, app) = setup(|_| {}).await;
    let mut c = Client::new(app.clone());
    let r = c.get("/api/claims").await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.headers.get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(r.headers.get("x-frame-options").unwrap(), "DENY");
    assert!(r.headers.contains_key("x-request-id"));
    let r = c
        .send(
            "POST",
            "/api/auth/login",
            Some(json!({"email": "a@b.co", "password": "x"})),
            false,
            &[("origin", "https://evil.example")],
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let h = c.get("/healthz").await;
    assert_eq!(h.body["status"], "ok");
    let ready = c.get("/readyz").await;
    assert_eq!(ready.status, StatusCode::OK);
}

#[tokio::test]
async fn weak_passwords_and_disabled_registration() {
    let (_s, app) = setup(|_| {}).await;
    let mut c = Client::new(app.clone());
    let r = c
        .send(
            "POST",
            "/api/auth/register",
            Some(json!({"email": "w@example.com", "name": "W", "password": "password"})),
            false,
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let (_s2, app2) = setup(|c| c.allow_registration = false).await;
    let mut c2 = Client::new(app2);
    let r = c2
        .send(
            "POST",
            "/api/auth/register",
            Some(json!({"email": "w@example.com", "name": "Wendy", "password": PASSWORD})),
            false,
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn deactivated_user_loses_access() {
    let (_s, app) = setup(|_| {}).await;
    let mut admin = login(&app, "admin@example.com", "Bootstrap-Strong-Pass-7").await;
    let mut u = register(&app, "gone@example.com").await;
    assert_eq!(u.get("/api/auth/me").await.status, StatusCode::OK);
    let users = admin.get("/api/admin/users").await;
    let id = users.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["email"] == "gone@example.com")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    admin
        .send(
            "PATCH",
            &format!("/api/admin/users/{id}"),
            Some(json!({"active": false})),
            true,
            &[],
        )
        .await;
    assert_eq!(u.get("/api/auth/me").await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn policy_search_stats_and_eval() {
    let (_s, app) = setup(|_| {}).await;
    let mut admin = login(&app, "admin@example.com", "Bootstrap-Strong-Pass-7").await;
    let r = admin
        .get("/api/policy/search?q=ambulance%20charges&k=3")
        .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["results"].as_array().unwrap().len(), 3);
    let etag = admin
        .get("/api/policy/chunks")
        .await
        .headers
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let r = admin
        .send(
            "GET",
            "/api/policy/chunks",
            None,
            false,
            &[("if-none-match", &etag)],
        )
        .await;
    assert_eq!(r.status, StatusCode::NOT_MODIFIED);
    admin
        .post("/api/claims", json!({"case": sample("PUB-008")}))
        .await;
    let s = admin.get("/api/stats").await;
    assert_eq!(s.body["stats"]["total_claims"], 1);
    let e = admin.post("/api/admin/eval", json!({})).await;
    assert_eq!(e.body["correct"], e.body["labelled_cases"]);
}
