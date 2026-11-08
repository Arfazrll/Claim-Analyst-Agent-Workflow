//! Claim Analyst Agent Workflow API.

pub mod auth;
pub mod cache;
pub mod config;
pub mod crypto;
pub mod db;
pub mod error;
pub mod ledger;
pub mod llm_http;
pub mod metrics;
pub mod middleware;
pub mod routes;
pub mod state;

pub use routes::router;
pub use state::AppState;

/// Single-line, control-character-free text for names and identifiers.
pub fn sanitize(s: &str, cap: usize) -> String {
    claim_engine::text_safety::clean(s, cap)
}

/// Free text that may keep line breaks (review notes): strips other control
/// characters and caps length. Rendered as text (never HTML) by the frontend.
pub fn sanitize_multiline(s: &str, cap: usize) -> String {
    s.chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .take(cap)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Create the bootstrap admin from `ADMIN_EMAIL` / `ADMIN_PASSWORD` when no user exists.
pub async fn bootstrap_admin(state: &AppState) -> Result<(), String> {
    let (Some(email), Some(password)) = (
        state.cfg.admin_email.clone(),
        state.cfg.admin_password.clone(),
    ) else {
        return Ok(());
    };
    if state.db.count_users().await.map_err(|e| e.to_string())? > 0 {
        return Ok(());
    }
    let email = email.trim().to_lowercase();
    if let Some(p) = auth::password_problems(&password, &email) {
        return Err(format!("ADMIN_PASSWORD rejected: {p}"));
    }
    let hash = auth::hash_password(&password).map_err(|e| e.to_string())?;
    match state
        .db
        .create_user(
            &uuid::Uuid::new_v4().to_string(),
            &email,
            "Administrator",
            &hash,
            "admin",
        )
        .await
    {
        Ok(_) => tracing::info!(email, "bootstrap admin created"),
        // Another replica created it first.
        Err(error::ApiError::Conflict(_)) => {
            tracing::info!(email, "bootstrap admin already exists")
        }
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}
