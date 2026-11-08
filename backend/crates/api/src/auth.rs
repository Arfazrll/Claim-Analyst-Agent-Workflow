//! Authentication and authorisation.
//!
//! * Passwords: Argon2id (OWASP parameters), constant-time verification, a dummy hash
//!   is verified for unknown emails so response timing does not reveal accounts.
//! * Sessions: 15-minute HS256 access JWT in an `HttpOnly; SameSite=Strict` cookie
//!   (or `Authorization: Bearer` for API clients), plus an opaque refresh token
//!   (stored hashed, rotated on every use, reuse revokes the whole family).
//! * CSRF: double-submit token. A random value is put in a readable `caw_csrf` cookie
//!   *and* inside the signed JWT; unsafe cookie-authenticated requests must echo it in
//!   `X-CSRF-Token`.
//! * RBAC: analyst < reviewer < admin.

use std::time::Duration;

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::Method;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use crate::config::Config;
use crate::crypto::{constant_time_eq, random_token};
use crate::db::now_ms;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub const ACCESS_COOKIE: &str = "caw_access";
pub const REFRESH_COOKIE: &str = "caw_refresh";
pub const CSRF_COOKIE: &str = "caw_csrf";
pub const CSRF_HEADER: &str = "x-csrf-token";
const ISSUER: &str = "claim-analyst-agent-workflow";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Analyst,
    Reviewer,
    Admin,
}

impl Role {
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "analyst" => Some(Role::Analyst),
            "reviewer" => Some(Role::Reviewer),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Analyst => "analyst",
            Role::Reviewer => "reviewer",
            Role::Admin => "admin",
        }
    }
}

// ------------------------------------------------------------------ passwords

static DUMMY_HASH: LazyLock<String> =
    LazyLock::new(|| hash_password("dummy-password-for-timing-equalisation").expect("hash"));

pub fn hash_password(password: &str) -> ApiResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(ApiError::internal)
}

pub fn verify_password(password: &str, hash: Option<&str>) -> bool {
    let target = hash.unwrap_or(DUMMY_HASH.as_str());
    let ok = PasswordHash::new(target)
        .map(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
        .unwrap_or(false);
    ok && hash.is_some()
}

/// Length >= 12, at least three character classes, not containing the email name.
pub fn password_problems(password: &str, email: &str) -> Option<String> {
    if password.chars().count() < 12 {
        return Some("Use at least 12 characters.".into());
    }
    if password.len() > 256 {
        return Some("Use at most 256 characters.".into());
    }
    let classes = [
        password.chars().any(|c| c.is_ascii_lowercase()),
        password.chars().any(|c| c.is_ascii_uppercase()),
        password.chars().any(|c| c.is_ascii_digit()),
        password.chars().any(|c| !c.is_ascii_alphanumeric()),
    ];
    if classes.iter().filter(|b| **b).count() < 3 {
        return Some("Mix at least three of: lowercase, uppercase, digits, symbols.".into());
    }
    let local = email.split('@').next().unwrap_or("").to_lowercase();
    if local.len() >= 4 && password.to_lowercase().contains(&local) {
        return Some("The password must not contain your email name.".into());
    }
    None
}

pub fn valid_email(email: &str) -> bool {
    let e = email.trim();
    e.len() <= 254
        && e.split_once('@').is_some_and(|(l, d)| {
            !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.')
        })
        && !e.contains(char::is_whitespace)
}

// ------------------------------------------------------------------ JWT

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub email: String,
    pub name: String,
    pub role: Role,
    pub csrf: String,
    pub iat: i64,
    pub exp: i64,
    pub iss: String,
    pub jti: String,
}

pub fn issue_access(
    cfg: &Config,
    user_id: &str,
    email: &str,
    name: &str,
    role: Role,
    csrf: &str,
) -> ApiResult<String> {
    let now = now_ms() / 1000;
    let claims = Claims {
        sub: user_id.into(),
        email: email.into(),
        name: name.into(),
        role,
        csrf: csrf.into(),
        iat: now,
        exp: now + cfg.access_ttl.as_secs() as i64,
        iss: ISSUER.into(),
        jti: uuid::Uuid::new_v4().to_string(),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&cfg.jwt_secret),
    )
    .map_err(ApiError::internal)
}

pub fn verify_access(cfg: &Config, token: &str) -> Option<Claims> {
    let mut v = Validation::new(Algorithm::HS256);
    v.set_issuer(&[ISSUER]);
    v.leeway = 5;
    decode::<Claims>(token, &DecodingKey::from_secret(&cfg.jwt_secret), &v)
        .ok()
        .map(|d| d.claims)
}

// ------------------------------------------------------------------ cookies

fn cookie(
    cfg: &Config,
    name: &'static str,
    value: String,
    path: &'static str,
    http_only: bool,
    ttl: Duration,
) -> Cookie<'static> {
    let mut c = Cookie::build((name, value))
        .path(path)
        .http_only(http_only)
        .secure(cfg.cookie_secure)
        .same_site(SameSite::Strict)
        .max_age(time::Duration::seconds(ttl.as_secs() as i64))
        .build();
    if let Some(d) = &cfg.cookie_domain {
        c.set_domain(d.clone());
    }
    c
}

pub fn session_cookies(
    cfg: &Config,
    jar: CookieJar,
    access: String,
    refresh: String,
    csrf: String,
) -> CookieJar {
    jar.add(cookie(
        cfg,
        ACCESS_COOKIE,
        access,
        "/",
        true,
        cfg.access_ttl,
    ))
    .add(cookie(
        cfg,
        REFRESH_COOKIE,
        refresh,
        "/api/auth",
        true,
        cfg.refresh_ttl,
    ))
    .add(cookie(cfg, CSRF_COOKIE, csrf, "/", false, cfg.refresh_ttl))
}

pub fn clear_cookies(cfg: &Config, jar: CookieJar) -> CookieJar {
    jar.add(cookie(
        cfg,
        ACCESS_COOKIE,
        String::new(),
        "/",
        true,
        Duration::ZERO,
    ))
    .add(cookie(
        cfg,
        REFRESH_COOKIE,
        String::new(),
        "/api/auth",
        true,
        Duration::ZERO,
    ))
    .add(cookie(
        cfg,
        CSRF_COOKIE,
        String::new(),
        "/",
        false,
        Duration::ZERO,
    ))
}

pub fn new_csrf() -> String {
    random_token(24)
}

// ------------------------------------------------------------------ extractor

/// An authenticated, active user.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: String,
    pub email: String,
    pub name: String,
    pub role: Role,
}

impl AuthUser {
    pub fn require(&self, role: Role) -> ApiResult<()> {
        if self.role >= role {
            Ok(())
        } else {
            Err(ApiError::Forbidden(format!(
                "This action needs the {} role.",
                role.as_str()
            )))
        }
    }
}

fn is_unsafe(m: &Method) -> bool {
    !matches!(*m, Method::GET | Method::HEAD | Method::OPTIONS)
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let bearer = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::to_string);
        let jar = CookieJar::from_headers(&parts.headers);
        let (token, via_cookie) = match bearer {
            Some(t) => (t, false),
            None => (
                jar.get(ACCESS_COOKIE)
                    .map(|c| c.value().to_string())
                    .ok_or(ApiError::Unauthorized)?,
                true,
            ),
        };
        let claims = verify_access(&state.cfg, &token).ok_or(ApiError::Unauthorized)?;

        if via_cookie && is_unsafe(&parts.method) {
            let header = parts
                .headers
                .get(CSRF_HEADER)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if header.is_empty() || !constant_time_eq(header, &claims.csrf) {
                return Err(ApiError::Forbidden("Missing or invalid CSRF token.".into()));
            }
        }

        // Deactivation and role changes take effect within the cache TTL, not at JWT expiry.
        let cache_key = format!("user:{}", claims.sub);
        let role = match state.cache.get(&cache_key).await {
            Some(v) if v == "inactive" => return Err(ApiError::Unauthorized),
            Some(v) => Role::parse(&v).unwrap_or(claims.role),
            None => {
                let user = state
                    .db
                    .user_by_id(&claims.sub)
                    .await?
                    .ok_or(ApiError::Unauthorized)?;
                let marker = if user.active {
                    user.role.clone()
                } else {
                    "inactive".into()
                };
                state
                    .cache
                    .set(&cache_key, &marker, Duration::from_secs(30))
                    .await;
                if !user.active {
                    return Err(ApiError::Unauthorized);
                }
                Role::parse(&user.role).unwrap_or(Role::Analyst)
            }
        };
        Ok(AuthUser {
            id: claims.sub,
            email: claims.email,
            name: claims.name,
            role,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_rules() {
        assert!(password_problems("short", "a@b.co").is_some());
        assert!(password_problems("alllowercaseletters", "a@b.co").is_some());
        assert!(password_problems("Johnathan-Secret-42", "johnathan@x.io").is_some());
        assert!(password_problems("Correct-Horse-Battery-9", "user@x.io").is_none());
    }

    #[test]
    fn hashing_roundtrip() {
        let h = hash_password("Correct-Horse-Battery-9").unwrap();
        assert!(verify_password("Correct-Horse-Battery-9", Some(&h)));
        assert!(!verify_password("wrong", Some(&h)));
        assert!(!verify_password("Correct-Horse-Battery-9", None));
    }

    #[test]
    fn jwt_roundtrip_and_tamper() {
        let cfg = Config::for_tests();
        let t = issue_access(&cfg, "u1", "a@b.co", "A", Role::Reviewer, "csrf").unwrap();
        let c = verify_access(&cfg, &t).unwrap();
        assert_eq!(c.role, Role::Reviewer);
        let mut bad = t.clone();
        bad.push('x');
        assert!(verify_access(&cfg, &bad).is_none());
    }

    #[test]
    fn email_check() {
        assert!(valid_email("a.b@example.com"));
        assert!(!valid_email("nope"));
        assert!(!valid_email("a@b"));
    }
}
