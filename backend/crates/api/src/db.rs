//! Persistence over SQLite (default, zero setup) or PostgreSQL (multi-replica), through
//! sqlx's `Any` driver with one portable schema: TEXT ids, BIGINT epoch-millisecond
//! timestamps, DOUBLE PRECISION amounts, `$n` placeholders (valid in both engines).
//!
//! Sensitive columns (`*_enc`) hold AES-256-GCM ciphertext; the rest are index fields
//! needed for listing and filtering.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sqlx::any::{AnyPoolOptions, AnyRow};
use sqlx::{AnyPool, Executor, Row};

use crate::error::{ApiError, ApiResult};

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DbKind {
    Sqlite,
    Postgres,
}

#[derive(Clone, Debug)]
pub struct Db {
    pub pool: AnyPool,
    pub kind: DbKind,
}

const MIGRATIONS: &[(i64, &str, &[&str])] = &[(
    1,
    "initial schema",
    &[
        "CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY, email TEXT NOT NULL UNIQUE, name TEXT NOT NULL, password_hash TEXT NOT NULL,
            role TEXT NOT NULL, active BIGINT NOT NULL DEFAULT 1, failed_logins BIGINT NOT NULL DEFAULT 0,
            locked_until BIGINT NOT NULL DEFAULT 0, created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS refresh_tokens (
            id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id), token_hash TEXT NOT NULL UNIQUE,
            family_id TEXT NOT NULL, expires_at BIGINT NOT NULL, revoked_at BIGINT, replaced_by TEXT,
            user_agent TEXT, ip TEXT, created_at BIGINT NOT NULL)",
        "CREATE INDEX IF NOT EXISTS idx_refresh_family ON refresh_tokens(family_id)",
        "CREATE TABLE IF NOT EXISTS claims (
            id TEXT PRIMARY KEY, case_id TEXT NOT NULL, policy_id TEXT, status TEXT NOT NULL, decision TEXT NOT NULL,
            final_decision TEXT, confidence DOUBLE PRECISION NOT NULL, claimed_total DOUBLE PRECISION NOT NULL,
            deductions DOUBLE PRECISION NOT NULL, payable DOUBLE PRECISION, payload_enc TEXT NOT NULL,
            analysis_enc TEXT NOT NULL, payload_hash TEXT NOT NULL, analysis_hash TEXT NOT NULL,
            engine_version TEXT NOT NULL, version BIGINT NOT NULL DEFAULT 1, created_by TEXT NOT NULL REFERENCES users(id),
            created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL)",
        "CREATE INDEX IF NOT EXISTS idx_claims_created ON claims(created_at)",
        "CREATE INDEX IF NOT EXISTS idx_claims_decision ON claims(decision)",
        "CREATE INDEX IF NOT EXISTS idx_claims_owner ON claims(created_by)",
        "CREATE TABLE IF NOT EXISTS reviews (
            id TEXT PRIMARY KEY, claim_id TEXT NOT NULL REFERENCES claims(id), reviewer_id TEXT NOT NULL REFERENCES users(id),
            action TEXT NOT NULL, final_decision TEXT NOT NULL, notes_enc TEXT NOT NULL, created_at BIGINT NOT NULL)",
        "CREATE INDEX IF NOT EXISTS idx_reviews_claim ON reviews(claim_id)",
        "CREATE TABLE IF NOT EXISTS ledger_head (id BIGINT PRIMARY KEY, seq BIGINT NOT NULL, head_hash TEXT NOT NULL)",
        "INSERT INTO ledger_head (id, seq, head_hash) VALUES (1, 0, '0000000000000000000000000000000000000000000000000000000000000000') ON CONFLICT (id) DO NOTHING",
        "CREATE TABLE IF NOT EXISTS ledger_entries (
            seq BIGINT PRIMARY KEY, claim_id TEXT NOT NULL, kind TEXT NOT NULL, payload_hash TEXT NOT NULL,
            prev_hash TEXT NOT NULL, entry_hash TEXT NOT NULL UNIQUE, actor_id TEXT NOT NULL, payload_enc TEXT NOT NULL, cid TEXT,
            chain_status TEXT NOT NULL, tx_hash TEXT, block_number BIGINT, attempts BIGINT NOT NULL DEFAULT 0,
            last_error TEXT, locked_until BIGINT NOT NULL DEFAULT 0, created_at BIGINT NOT NULL, anchored_at BIGINT)",
        "CREATE INDEX IF NOT EXISTS idx_ledger_claim ON ledger_entries(claim_id)",
        "CREATE INDEX IF NOT EXISTS idx_ledger_status ON ledger_entries(chain_status)",
        "CREATE TABLE IF NOT EXISTS audit_log (
            id TEXT PRIMARY KEY, actor_id TEXT, action TEXT NOT NULL, target TEXT, ip TEXT, detail TEXT, created_at BIGINT NOT NULL)",
        "CREATE INDEX IF NOT EXISTS idx_audit_created ON audit_log(created_at)",
    ],
)];

impl Db {
    pub async fn connect(url: &str) -> ApiResult<Db> {
        sqlx::any::install_default_drivers();
        let kind = if url.starts_with("postgres") {
            DbKind::Postgres
        } else {
            DbKind::Sqlite
        };
        if kind == DbKind::Sqlite {
            if let Some(path) = url
                .strip_prefix("sqlite://")
                .map(|p| p.split('?').next().unwrap_or(p))
            {
                if let Some(parent) = std::path::Path::new(path).parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent).map_err(ApiError::internal)?;
                    }
                }
            }
        }
        let memory = url.contains(":memory:");
        let mut opts = AnyPoolOptions::new()
            .max_connections(match (kind, memory) {
                (_, true) => 1,
                (DbKind::Sqlite, false) => 8,
                (DbKind::Postgres, false) => 20,
            })
            .acquire_timeout(std::time::Duration::from_secs(10));
        if kind == DbKind::Sqlite {
            opts = opts.after_connect(|conn, _meta| {
                Box::pin(async move {
                    conn.execute("PRAGMA foreign_keys = ON").await?;
                    conn.execute("PRAGMA busy_timeout = 5000").await?;
                    conn.execute("PRAGMA synchronous = NORMAL").await?;
                    Ok(())
                })
            });
        }
        let pool = opts.connect(url).await.map_err(ApiError::internal)?;
        if kind == DbKind::Sqlite && !memory {
            sqlx::query("PRAGMA journal_mode = WAL")
                .execute(&pool)
                .await
                .map_err(ApiError::internal)?;
        }
        let db = Db { pool, kind };
        db.migrate().await?;
        Ok(db)
    }

    /// Apply pending migrations on one connection. On Postgres an advisory lock makes
    /// replicas that start at the same time apply them one after another.
    async fn migrate(&self) -> ApiResult<()> {
        use sqlx::Connection;
        let mut conn = self.pool.acquire().await?;
        if self.kind == DbKind::Postgres {
            sqlx::query("SELECT pg_advisory_lock(7262701)")
                .execute(&mut *conn)
                .await?;
        }
        let result: ApiResult<()> = async {
            sqlx::query("CREATE TABLE IF NOT EXISTS schema_migrations (version BIGINT PRIMARY KEY, name TEXT NOT NULL, applied_at BIGINT NOT NULL)")
                .execute(&mut *conn)
                .await?;
            for (version, name, statements) in MIGRATIONS {
                let done = sqlx::query("SELECT version FROM schema_migrations WHERE version = $1").bind(*version).fetch_optional(&mut *conn).await?;
                if done.is_some() {
                    continue;
                }
                let mut tx = conn.begin().await?;
                for stmt in *statements {
                    sqlx::query(stmt).execute(&mut *tx).await?;
                }
                sqlx::query("INSERT INTO schema_migrations (version, name, applied_at) VALUES ($1, $2, $3) ON CONFLICT (version) DO NOTHING")
                    .bind(*version)
                    .bind(*name)
                    .bind(now_ms())
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                tracing::info!(version, name, "applied migration");
            }
            Ok(())
        }
        .await;
        if self.kind == DbKind::Postgres {
            let _ = sqlx::query("SELECT pg_advisory_unlock(7262701)")
                .execute(&mut *conn)
                .await;
        }
        result
    }

    pub async fn ping(&self) -> bool {
        sqlx::query("SELECT 1").fetch_one(&self.pool).await.is_ok()
    }
}

// ------------------------------------------------------------------ users

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: String,
    #[serde(skip)]
    pub password_hash: String,
    pub role: String,
    pub active: bool,
    #[serde(skip)]
    pub failed_logins: i64,
    #[serde(skip)]
    pub locked_until: i64,
    pub created_at: i64,
}

fn user_from(r: &AnyRow) -> User {
    User {
        id: r.get("id"),
        email: r.get("email"),
        name: r.get("name"),
        password_hash: r.get("password_hash"),
        role: r.get("role"),
        active: r.get::<i64, _>("active") != 0,
        failed_logins: r.get("failed_logins"),
        locked_until: r.get("locked_until"),
        created_at: r.get("created_at"),
    }
}

const USER_COLS: &str =
    "id, email, name, password_hash, role, active, failed_logins, locked_until, created_at";

impl Db {
    pub async fn count_users(&self) -> ApiResult<i64> {
        Ok(sqlx::query("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await?
            .get::<i64, _>(0))
    }

    pub async fn create_user(
        &self,
        id: &str,
        email: &str,
        name: &str,
        hash: &str,
        role: &str,
    ) -> ApiResult<User> {
        let now = now_ms();
        sqlx::query("INSERT INTO users (id, email, name, password_hash, role, active, failed_logins, locked_until, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, 1, 0, 0, $6, $7)")
            .bind(id).bind(email).bind(name).bind(hash).bind(role).bind(now).bind(now)
            .execute(&self.pool)
            .await?;
        self.user_by_id(id).await?.ok_or(ApiError::NotFound)
    }

    pub async fn user_by_email(&self, email: &str) -> ApiResult<Option<User>> {
        let row = sqlx::query(&format!("SELECT {USER_COLS} FROM users WHERE email = $1"))
            .bind(email)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(user_from))
    }

    pub async fn user_by_id(&self, id: &str) -> ApiResult<Option<User>> {
        let row = sqlx::query(&format!("SELECT {USER_COLS} FROM users WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(user_from))
    }

    pub async fn list_users(&self) -> ApiResult<Vec<User>> {
        let rows = sqlx::query(&format!(
            "SELECT {USER_COLS} FROM users ORDER BY created_at"
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(user_from).collect())
    }

    pub async fn update_user_admin(&self, id: &str, role: &str, active: bool) -> ApiResult<()> {
        let r =
            sqlx::query("UPDATE users SET role = $1, active = $2, updated_at = $3 WHERE id = $4")
                .bind(role)
                .bind(active as i64)
                .bind(now_ms())
                .bind(id)
                .execute(&self.pool)
                .await?;
        if r.rows_affected() == 0 {
            return Err(ApiError::NotFound);
        }
        Ok(())
    }

    pub async fn update_password(&self, id: &str, hash: &str) -> ApiResult<()> {
        sqlx::query("UPDATE users SET password_hash = $1, updated_at = $2 WHERE id = $3")
            .bind(hash)
            .bind(now_ms())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Records a failed sign-in; locks the account for `lock_ms` after `max` failures.
    pub async fn record_failed_login(&self, id: &str, max: i64, lock_ms: i64) -> ApiResult<()> {
        let now = now_ms();
        sqlx::query(
            "UPDATE users SET failed_logins = failed_logins + 1,
                locked_until = CASE WHEN failed_logins + 1 >= $1 THEN $2 ELSE locked_until END,
                updated_at = $3 WHERE id = $4",
        )
        .bind(max)
        .bind(now + lock_ms)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn reset_failed_logins(&self, id: &str) -> ApiResult<()> {
        sqlx::query("UPDATE users SET failed_logins = 0, locked_until = 0 WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

// ------------------------------------------------------------------ refresh tokens

#[derive(Debug, Clone)]
pub struct RefreshToken {
    pub id: String,
    pub user_id: String,
    pub family_id: String,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
}

impl Db {
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_refresh(
        &self,
        id: &str,
        user_id: &str,
        hash: &str,
        family: &str,
        expires_at: i64,
        ua: Option<&str>,
        ip: Option<&str>,
    ) -> ApiResult<()> {
        sqlx::query("INSERT INTO refresh_tokens (id, user_id, token_hash, family_id, expires_at, user_agent, ip, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
            .bind(id).bind(user_id).bind(hash).bind(family).bind(expires_at)
            .bind(ua.map(|s| s.chars().take(200).collect::<String>())).bind(ip.map(str::to_string)).bind(now_ms())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn refresh_by_hash(&self, hash: &str) -> ApiResult<Option<RefreshToken>> {
        let row = sqlx::query("SELECT id, user_id, family_id, expires_at, revoked_at FROM refresh_tokens WHERE token_hash = $1")
            .bind(hash)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| RefreshToken {
            id: r.get("id"),
            user_id: r.get("user_id"),
            family_id: r.get("family_id"),
            expires_at: r.get("expires_at"),
            revoked_at: r.get("revoked_at"),
        }))
    }

    /// Marks a token used (rotation). Returns false when it was already revoked,
    /// which signals refresh-token reuse.
    pub async fn rotate_refresh(&self, id: &str, replaced_by: &str) -> ApiResult<bool> {
        let r = sqlx::query("UPDATE refresh_tokens SET revoked_at = $1, replaced_by = $2 WHERE id = $3 AND revoked_at IS NULL")
            .bind(now_ms()).bind(replaced_by).bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn revoke_family(&self, family: &str) -> ApiResult<()> {
        sqlx::query(
            "UPDATE refresh_tokens SET revoked_at = $1 WHERE family_id = $2 AND revoked_at IS NULL",
        )
        .bind(now_ms())
        .bind(family)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn revoke_user_tokens(&self, user_id: &str) -> ApiResult<()> {
        sqlx::query(
            "UPDATE refresh_tokens SET revoked_at = $1 WHERE user_id = $2 AND revoked_at IS NULL",
        )
        .bind(now_ms())
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

// ------------------------------------------------------------------ claims

#[derive(Debug, Clone, Serialize)]
pub struct ClaimRow {
    pub id: String,
    pub case_id: String,
    pub policy_id: Option<String>,
    pub status: String,
    pub decision: String,
    pub final_decision: Option<String>,
    pub confidence: f64,
    pub claimed_total: f64,
    pub deductions: f64,
    pub payable: Option<f64>,
    #[serde(skip)]
    pub payload_enc: String,
    #[serde(skip)]
    pub analysis_enc: String,
    pub payload_hash: String,
    pub analysis_hash: String,
    pub engine_version: String,
    pub version: i64,
    pub created_by: String,
    pub created_by_name: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

const CLAIM_COLS: &str = "c.id, c.case_id, c.policy_id, c.status, c.decision, c.final_decision, c.confidence, c.claimed_total, c.deductions, c.payable, c.payload_enc, c.analysis_enc, c.payload_hash, c.analysis_hash, c.engine_version, c.version, c.created_by, u.name AS created_by_name, c.created_at, c.updated_at";

fn claim_from(r: &AnyRow) -> ClaimRow {
    ClaimRow {
        id: r.get("id"),
        case_id: r.get("case_id"),
        policy_id: r.get("policy_id"),
        status: r.get("status"),
        decision: r.get("decision"),
        final_decision: r.get("final_decision"),
        confidence: r.get("confidence"),
        claimed_total: r.get("claimed_total"),
        deductions: r.get("deductions"),
        payable: r.get("payable"),
        payload_enc: r.get("payload_enc"),
        analysis_enc: r.get("analysis_enc"),
        payload_hash: r.get("payload_hash"),
        analysis_hash: r.get("analysis_hash"),
        engine_version: r.get("engine_version"),
        version: r.get("version"),
        created_by: r.get("created_by"),
        created_by_name: r.get("created_by_name"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaimFilter {
    pub decision: Option<String>,
    pub status: Option<String>,
    pub search: Option<String>,
    pub owner: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub total_claims: i64,
    pub by_decision: Vec<(String, i64)>,
    pub by_status: Vec<(String, i64)>,
    pub claimed_total: f64,
    pub payable_total: f64,
    pub ledger_entries: i64,
    pub ledger_by_status: Vec<(String, i64)>,
}

pub struct NewClaim<'a> {
    pub id: &'a str,
    pub case_id: &'a str,
    pub policy_id: Option<&'a str>,
    pub decision: &'a str,
    pub confidence: f64,
    pub claimed_total: f64,
    pub deductions: f64,
    pub payable: Option<f64>,
    pub payload_enc: &'a str,
    pub analysis_enc: &'a str,
    pub payload_hash: &'a str,
    pub analysis_hash: &'a str,
    pub engine_version: &'a str,
    pub created_by: &'a str,
}

impl Db {
    pub async fn insert_claim(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
        c: &NewClaim<'_>,
    ) -> ApiResult<()> {
        let now = now_ms();
        sqlx::query(
            "INSERT INTO claims (id, case_id, policy_id, status, decision, final_decision, confidence, claimed_total, deductions, payable,
                payload_enc, analysis_enc, payload_hash, analysis_hash, engine_version, version, created_by, created_at, updated_at)
             VALUES ($1, $2, $3, 'analyzed', $4, NULL, $5, $6, $7, $8, $9, $10, $11, $12, $13, 1, $14, $15, $16)",
        )
        .bind(c.id).bind(c.case_id).bind(c.policy_id.map(str::to_string)).bind(c.decision).bind(c.confidence)
        .bind(c.claimed_total).bind(c.deductions).bind(c.payable).bind(c.payload_enc).bind(c.analysis_enc)
        .bind(c.payload_hash).bind(c.analysis_hash).bind(c.engine_version).bind(c.created_by).bind(now).bind(now)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn update_claim_analysis(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
        c: &NewClaim<'_>,
    ) -> ApiResult<i64> {
        let row = sqlx::query(
            "UPDATE claims SET status = 'analyzed', decision = $1, final_decision = NULL, confidence = $2, claimed_total = $3,
                deductions = $4, payable = $5, analysis_enc = $6, analysis_hash = $7, engine_version = $8,
                version = version + 1, updated_at = $9 WHERE id = $10 RETURNING version",
        )
        .bind(c.decision).bind(c.confidence).bind(c.claimed_total).bind(c.deductions).bind(c.payable)
        .bind(c.analysis_enc).bind(c.analysis_hash).bind(c.engine_version).bind(now_ms()).bind(c.id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(ApiError::NotFound)?;
        Ok(row.get::<i64, _>(0))
    }

    pub async fn set_claim_review(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
        id: &str,
        status: &str,
        final_decision: &str,
    ) -> ApiResult<()> {
        sqlx::query(
            "UPDATE claims SET status = $1, final_decision = $2, updated_at = $3 WHERE id = $4",
        )
        .bind(status)
        .bind(final_decision)
        .bind(now_ms())
        .bind(id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn claim(&self, id: &str) -> ApiResult<Option<ClaimRow>> {
        let row = sqlx::query(&format!("SELECT {CLAIM_COLS} FROM claims c LEFT JOIN users u ON u.id = c.created_by WHERE c.id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(claim_from))
    }

    pub async fn list_claims(&self, f: &ClaimFilter) -> ApiResult<(Vec<ClaimRow>, i64)> {
        let mut where_ = vec!["1 = 1".to_string()];
        let mut binds: Vec<String> = vec![];
        let mut push = |cond: &str, v: String, binds: &mut Vec<String>| {
            binds.push(v);
            where_.push(cond.replace('?', &format!("${}", binds.len())));
        };
        if let Some(d) = &f.decision {
            push("c.decision = ?", d.clone(), &mut binds);
        }
        if let Some(s) = &f.status {
            push("c.status = ?", s.clone(), &mut binds);
        }
        if let Some(o) = &f.owner {
            push("c.created_by = ?", o.clone(), &mut binds);
        }
        if let Some(q) = &f.search {
            let escaped = q
                .to_lowercase()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            push(
                "LOWER(c.case_id) LIKE ? ESCAPE '\\'",
                format!("%{escaped}%"),
                &mut binds,
            );
        }
        let clause = where_.join(" AND ");
        let count_sql = format!("SELECT COUNT(*) FROM claims c WHERE {clause}");
        let mut count_q = sqlx::query(&count_sql);
        for b in &binds {
            count_q = count_q.bind(b.clone());
        }
        let total: i64 = count_q.fetch_one(&self.pool).await?.get(0);
        let sql = format!(
            "SELECT {CLAIM_COLS} FROM claims c LEFT JOIN users u ON u.id = c.created_by WHERE {clause} ORDER BY c.created_at DESC LIMIT ${} OFFSET ${}",
            binds.len() + 1,
            binds.len() + 2
        );
        let mut q = sqlx::query(&sql);
        for b in &binds {
            q = q.bind(b.clone());
        }
        let rows = q.bind(f.limit).bind(f.offset).fetch_all(&self.pool).await?;
        Ok((rows.iter().map(claim_from).collect(), total))
    }

    async fn rows(&self, sql: &str, bind: Option<&str>) -> ApiResult<Vec<AnyRow>> {
        let mut q = sqlx::query(sql);
        if let Some(b) = bind {
            q = q.bind(b.to_string());
        }
        Ok(q.fetch_all(&self.pool).await?)
    }

    pub async fn stats(&self, owner: Option<&str>) -> ApiResult<Stats> {
        let cond = if owner.is_some() {
            "WHERE created_by = $1"
        } else {
            ""
        };
        let pairs = |rows: Vec<AnyRow>| {
            rows.iter()
                .map(|r| (r.get::<String, _>(0), r.get::<i64, _>(1)))
                .collect::<Vec<_>>()
        };
        let total: i64 = self
            .rows(&format!("SELECT COUNT(*) FROM claims {cond}"), owner)
            .await?
            .first()
            .map(|r| r.get(0))
            .unwrap_or(0);
        let by_decision = pairs(self.rows(&format!("SELECT decision, COUNT(*) FROM claims {cond} GROUP BY decision ORDER BY decision"), owner).await?);
        let by_status = pairs(
            self.rows(
                &format!(
                    "SELECT status, COUNT(*) FROM claims {cond} GROUP BY status ORDER BY status"
                ),
                owner,
            )
            .await?,
        );
        // Amounts are summed in Rust: SUM() result types differ between SQLite and Postgres.
        let amounts = self
            .rows(
                &format!("SELECT claimed_total, payable FROM claims {cond}"),
                owner,
            )
            .await?;
        let claimed_total = amounts
            .iter()
            .map(|r| r.get::<f64, _>(0))
            .fold(0.0, |a, b| a + b);
        let payable_total = amounts
            .iter()
            .filter_map(|r| r.get::<Option<f64>, _>(1))
            .fold(0.0, |a, b| a + b);
        let ledger_entries: i64 = sqlx::query("SELECT COUNT(*) FROM ledger_entries")
            .fetch_one(&self.pool)
            .await?
            .get(0);
        let ledger_by_status = pairs(self.rows("SELECT chain_status, COUNT(*) FROM ledger_entries GROUP BY chain_status ORDER BY chain_status", None).await?);
        Ok(Stats {
            total_claims: total,
            by_decision,
            by_status,
            claimed_total,
            payable_total,
            ledger_entries,
            ledger_by_status,
        })
    }
}

// ------------------------------------------------------------------ reviews

#[derive(Debug, Clone, Serialize)]
pub struct ReviewRow {
    pub id: String,
    pub claim_id: String,
    pub reviewer_id: String,
    pub reviewer_name: Option<String>,
    pub action: String,
    pub final_decision: String,
    #[serde(skip)]
    pub notes_enc: String,
    pub created_at: i64,
}

impl Db {
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_review(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
        id: &str,
        claim_id: &str,
        reviewer: &str,
        action: &str,
        final_decision: &str,
        notes_enc: &str,
    ) -> ApiResult<()> {
        sqlx::query("INSERT INTO reviews (id, claim_id, reviewer_id, action, final_decision, notes_enc, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)")
            .bind(id).bind(claim_id).bind(reviewer).bind(action).bind(final_decision).bind(notes_enc).bind(now_ms())
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    pub async fn reviews(&self, claim_id: &str) -> ApiResult<Vec<ReviewRow>> {
        let rows = sqlx::query(
            "SELECT r.id, r.claim_id, r.reviewer_id, u.name AS reviewer_name, r.action, r.final_decision, r.notes_enc, r.created_at
             FROM reviews r LEFT JOIN users u ON u.id = r.reviewer_id WHERE r.claim_id = $1 ORDER BY r.created_at",
        )
        .bind(claim_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| ReviewRow {
                id: r.get("id"),
                claim_id: r.get("claim_id"),
                reviewer_id: r.get("reviewer_id"),
                reviewer_name: r.get("reviewer_name"),
                action: r.get("action"),
                final_decision: r.get("final_decision"),
                notes_enc: r.get("notes_enc"),
                created_at: r.get("created_at"),
            })
            .collect())
    }
}

// ------------------------------------------------------------------ ledger

#[derive(Debug, Clone, Serialize)]
pub struct LedgerEntry {
    pub seq: i64,
    pub claim_id: String,
    pub kind: String,
    pub payload_hash: String,
    pub prev_hash: String,
    pub entry_hash: String,
    pub actor_id: String,
    #[serde(skip)]
    pub payload_enc: String,
    pub cid: Option<String>,
    pub chain_status: String,
    pub tx_hash: Option<String>,
    pub block_number: Option<i64>,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub anchored_at: Option<i64>,
}

const LEDGER_COLS: &str = "seq, claim_id, kind, payload_hash, prev_hash, entry_hash, actor_id, payload_enc, cid, chain_status, tx_hash, block_number, attempts, last_error, created_at, anchored_at";

fn ledger_from(r: &AnyRow) -> LedgerEntry {
    LedgerEntry {
        seq: r.get("seq"),
        claim_id: r.get("claim_id"),
        kind: r.get("kind"),
        payload_hash: r.get("payload_hash"),
        prev_hash: r.get("prev_hash"),
        entry_hash: r.get("entry_hash"),
        actor_id: r.get("actor_id"),
        payload_enc: r.get("payload_enc"),
        cid: r.get("cid"),
        chain_status: r.get("chain_status"),
        tx_hash: r.get("tx_hash"),
        block_number: r.get("block_number"),
        attempts: r.get("attempts"),
        last_error: r.get("last_error"),
        created_at: r.get("created_at"),
        anchored_at: r.get("anchored_at"),
    }
}

impl Db {
    /// Reserve the next sequence number and read the current head hash. The UPDATE
    /// takes the head row lock (Postgres) / the write lock (SQLite) until commit, so
    /// appends are serialised across replicas.
    pub async fn ledger_reserve(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
    ) -> ApiResult<(i64, String)> {
        let row = sqlx::query(
            "UPDATE ledger_head SET seq = seq + 1 WHERE id = 1 RETURNING seq, head_hash",
        )
        .fetch_one(&mut **tx)
        .await?;
        Ok((row.get::<i64, _>(0), row.get::<String, _>(1)))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn ledger_insert(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
        e: &LedgerEntry,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO ledger_entries (seq, claim_id, kind, payload_hash, prev_hash, entry_hash, actor_id, payload_enc, chain_status, attempts, locked_until, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0, 0, $10)",
        )
        .bind(e.seq).bind(e.claim_id.clone()).bind(e.kind.clone()).bind(e.payload_hash.clone()).bind(e.prev_hash.clone())
        .bind(e.entry_hash.clone()).bind(e.actor_id.clone()).bind(e.payload_enc.clone()).bind(e.chain_status.clone()).bind(e.created_at)
        .execute(&mut **tx)
        .await?;
        sqlx::query("UPDATE ledger_head SET head_hash = $1 WHERE id = 1")
            .bind(e.entry_hash.clone())
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    pub async fn ledger_for_claim(&self, claim_id: &str) -> ApiResult<Vec<LedgerEntry>> {
        let rows = sqlx::query(&format!(
            "SELECT {LEDGER_COLS} FROM ledger_entries WHERE claim_id = $1 ORDER BY seq"
        ))
        .bind(claim_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(ledger_from).collect())
    }

    pub async fn ledger_page(&self, limit: i64, offset: i64) -> ApiResult<(Vec<LedgerEntry>, i64)> {
        let total: i64 = sqlx::query("SELECT COUNT(*) FROM ledger_entries")
            .fetch_one(&self.pool)
            .await?
            .get(0);
        let rows = sqlx::query(&format!(
            "SELECT {LEDGER_COLS} FROM ledger_entries ORDER BY seq DESC LIMIT $1 OFFSET $2"
        ))
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok((rows.iter().map(ledger_from).collect(), total))
    }

    pub async fn ledger_range(&self, after_seq: i64, limit: i64) -> ApiResult<Vec<LedgerEntry>> {
        let rows = sqlx::query(&format!(
            "SELECT {LEDGER_COLS} FROM ledger_entries WHERE seq > $1 ORDER BY seq LIMIT $2"
        ))
        .bind(after_seq)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(ledger_from).collect())
    }

    pub async fn ledger_by_seq(&self, seq: i64) -> ApiResult<Option<LedgerEntry>> {
        let row = sqlx::query(&format!(
            "SELECT {LEDGER_COLS} FROM ledger_entries WHERE seq = $1"
        ))
        .bind(seq)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.as_ref().map(ledger_from))
    }

    pub async fn ledger_head(&self) -> ApiResult<(i64, String)> {
        let r = sqlx::query("SELECT seq, head_hash FROM ledger_head WHERE id = 1")
            .fetch_one(&self.pool)
            .await?;
        Ok((r.get(0), r.get(1)))
    }

    pub async fn ledger_pending(&self, limit: i64) -> ApiResult<Vec<LedgerEntry>> {
        let rows = sqlx::query(&format!(
            "SELECT {LEDGER_COLS} FROM ledger_entries WHERE chain_status IN ('pending', 'retry') AND locked_until < $1 ORDER BY seq LIMIT $2"
        ))
        .bind(now_ms())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(ledger_from).collect())
    }

    /// Optimistic lease so only one replica anchors an entry at a time.
    pub async fn ledger_lease(&self, seq: i64, lease_ms: i64) -> ApiResult<bool> {
        let now = now_ms();
        let r = sqlx::query("UPDATE ledger_entries SET locked_until = $1 WHERE seq = $2 AND locked_until < $3 AND chain_status IN ('pending', 'retry')")
            .bind(now + lease_ms)
            .bind(seq)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn ledger_set_cid(&self, seq: i64, cid: &str) -> ApiResult<()> {
        sqlx::query("UPDATE ledger_entries SET cid = $1 WHERE seq = $2")
            .bind(cid)
            .bind(seq)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn ledger_mark(
        &self,
        seq: i64,
        status: &str,
        tx_hash: Option<&str>,
        block: Option<i64>,
    ) -> ApiResult<()> {
        sqlx::query("UPDATE ledger_entries SET chain_status = $1, tx_hash = $2, block_number = $3, anchored_at = $4, last_error = NULL, locked_until = 0 WHERE seq = $5")
            .bind(status)
            .bind(tx_hash.map(str::to_string))
            .bind(block)
            .bind(now_ms())
            .bind(seq)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn ledger_fail(
        &self,
        seq: i64,
        error: &str,
        retry_after_ms: i64,
        give_up_after: i64,
    ) -> ApiResult<()> {
        sqlx::query(
            "UPDATE ledger_entries SET attempts = attempts + 1, last_error = $1, locked_until = $2,
                chain_status = CASE WHEN attempts + 1 >= $3 THEN 'failed' ELSE 'retry' END WHERE seq = $4",
        )
        .bind(error.chars().take(500).collect::<String>())
        .bind(now_ms() + retry_after_ms)
        .bind(give_up_after)
        .bind(seq)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn ledger_requeue_failed(&self) -> ApiResult<u64> {
        let r = sqlx::query("UPDATE ledger_entries SET chain_status = 'retry', attempts = 0, locked_until = 0 WHERE chain_status = 'failed'").execute(&self.pool).await?;
        Ok(r.rows_affected())
    }
}

// ------------------------------------------------------------------ audit

#[derive(Debug, Clone, Serialize)]
pub struct AuditRow {
    pub id: String,
    pub actor_id: Option<String>,
    pub actor_email: Option<String>,
    pub action: String,
    pub target: Option<String>,
    pub ip: Option<String>,
    pub detail: Option<String>,
    pub created_at: i64,
}

impl Db {
    pub async fn audit(
        &self,
        actor: Option<&str>,
        action: &str,
        target: Option<&str>,
        ip: Option<&str>,
        detail: Option<String>,
    ) {
        let res = sqlx::query("INSERT INTO audit_log (id, actor_id, action, target, ip, detail, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)")
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(actor.map(str::to_string))
            .bind(action)
            .bind(target.map(str::to_string))
            .bind(ip.map(str::to_string))
            .bind(detail.map(|d| d.chars().take(1000).collect::<String>()))
            .bind(now_ms())
            .execute(&self.pool)
            .await;
        if let Err(e) = res {
            tracing::warn!(error = %e, action, "failed to write audit log");
        }
    }

    pub async fn audit_page(&self, limit: i64, offset: i64) -> ApiResult<(Vec<AuditRow>, i64)> {
        let total: i64 = sqlx::query("SELECT COUNT(*) FROM audit_log")
            .fetch_one(&self.pool)
            .await?
            .get(0);
        let rows = sqlx::query(
            "SELECT a.id, a.actor_id, u.email AS actor_email, a.action, a.target, a.ip, a.detail, a.created_at
             FROM audit_log a LEFT JOIN users u ON u.id = a.actor_id ORDER BY a.created_at DESC LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok((
            rows.iter()
                .map(|r| AuditRow {
                    id: r.get("id"),
                    actor_id: r.get("actor_id"),
                    actor_email: r.get("actor_email"),
                    action: r.get("action"),
                    target: r.get("target"),
                    ip: r.get("ip"),
                    detail: r.get("detail"),
                    created_at: r.get("created_at"),
                })
                .collect(),
            total,
        ))
    }
}
