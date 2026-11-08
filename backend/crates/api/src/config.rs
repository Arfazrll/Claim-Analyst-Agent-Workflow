//! Runtime configuration from environment variables (12-factor). Secrets never have
//! insecure defaults in production: startup fails if they are missing.

use std::time::Duration;

use base64::Engine as _;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEnv {
    Development,
    Production,
    Test,
}

#[derive(Clone)]
pub struct Config {
    pub env: AppEnv,
    pub bind_addr: String,
    pub database_url: String,
    pub redis_url: Option<String>,
    pub jwt_secret: Vec<u8>,
    pub data_key: [u8; 32],
    pub cors_origins: Vec<String>,
    pub cookie_secure: bool,
    pub cookie_domain: Option<String>,
    pub access_ttl: Duration,
    pub refresh_ttl: Duration,
    pub trust_proxy: bool,
    pub allow_registration: bool,
    pub admin_email: Option<String>,
    pub admin_password: Option<String>,
    pub metrics_token: Option<String>,
    pub cache_ttl: Duration,
    pub rate_auth_per_min: u64,
    pub rate_analyze_per_min: u64,
    pub rate_api_per_min: u64,
    pub request_timeout: Duration,
    pub body_limit_bytes: usize,
    // LLM (optional, OpenAI-compatible)
    pub llm_provider: String,
    pub llm_api_key: Option<String>,
    pub llm_base_url: String,
    pub llm_model: String,
    pub llm_interpretation_model: String,
    pub llm_interpretation_default: bool,
    // Web3
    pub chain_enabled: bool,
    pub chain_rpc_url: String,
    pub chain_id: u64,
    pub chain_private_key: Option<String>,
    pub chain_contract_address: Option<String>,
    pub chain_deployment_file: Option<String>,
    pub chain_explorer_url: Option<String>,
    pub ipfs_enabled: bool,
    pub ipfs_api_url: String,
    pub ipfs_gateway_url: String,
    pub anchor_interval: Duration,
    pub worker_enabled: bool,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print secrets.
        f.debug_struct("Config")
            .field("env", &self.env)
            .field("bind_addr", &self.bind_addr)
            .field("database", &self.database_url.split(':').next())
            .field("redis", &self.redis_url.is_some())
            .field("chain_enabled", &self.chain_enabled)
            .field("ipfs_enabled", &self.ipfs_enabled)
            .field("llm_provider", &self.llm_provider)
            .finish_non_exhaustive()
    }
}

fn var(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn flag(key: &str, default: bool) -> bool {
    var(key)
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

fn num<T: std::str::FromStr>(key: &str, default: T) -> T {
    var(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} must be set in production")]
    Missing(&'static str),
    #[error("{0}: {1}")]
    Invalid(&'static str, String),
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let env = match var("APP_ENV").as_deref() {
            Some("production") | Some("prod") => AppEnv::Production,
            Some("test") => AppEnv::Test,
            _ => AppEnv::Development,
        };
        let prod = env == AppEnv::Production;

        let jwt_secret = match var("JWT_SECRET") {
            Some(s) if s.len() >= 32 => s.into_bytes(),
            Some(_) => {
                return Err(ConfigError::Invalid(
                    "JWT_SECRET",
                    "must be at least 32 characters".into(),
                ))
            }
            None if prod => return Err(ConfigError::Missing("JWT_SECRET")),
            None => b"dev-only-insecure-jwt-secret-change-me-0123456789".to_vec(),
        };
        let data_key = match var("DATA_ENCRYPTION_KEY") {
            Some(k) => {
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(k.as_bytes())
                    .map_err(|e| ConfigError::Invalid("DATA_ENCRYPTION_KEY", e.to_string()))?;
                raw.try_into().map_err(|_| {
                    ConfigError::Invalid(
                        "DATA_ENCRYPTION_KEY",
                        "must decode to exactly 32 bytes".into(),
                    )
                })?
            }
            None if prod => return Err(ConfigError::Missing("DATA_ENCRYPTION_KEY")),
            None => *b"dev-only-insecure-data-key-32byt",
        };
        let cors_origins = var("CORS_ORIGINS")
            .unwrap_or_else(|| "http://localhost:5173,http://localhost:8080".into())
            .split(',')
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(Config {
            bind_addr: var("BIND_ADDR").unwrap_or_else(|| "0.0.0.0:8000".into()),
            database_url: var("DATABASE_URL")
                .unwrap_or_else(|| "sqlite://data/claims.db?mode=rwc".into()),
            redis_url: var("REDIS_URL"),
            jwt_secret,
            data_key,
            cors_origins,
            cookie_secure: flag("COOKIE_SECURE", prod),
            cookie_domain: var("COOKIE_DOMAIN"),
            access_ttl: Duration::from_secs(num("ACCESS_TOKEN_TTL_SECS", 900)),
            refresh_ttl: Duration::from_secs(num("REFRESH_TOKEN_TTL_SECS", 7 * 24 * 3600)),
            trust_proxy: flag("TRUST_PROXY", false),
            allow_registration: flag("ALLOW_REGISTRATION", true),
            admin_email: var("ADMIN_EMAIL"),
            admin_password: var("ADMIN_PASSWORD"),
            metrics_token: var("METRICS_TOKEN"),
            cache_ttl: Duration::from_secs(num("CACHE_TTL_SECS", 3600)),
            rate_auth_per_min: num("RATE_LIMIT_AUTH_PER_MIN", 10),
            rate_analyze_per_min: num("RATE_LIMIT_ANALYZE_PER_MIN", 30),
            rate_api_per_min: num("RATE_LIMIT_API_PER_MIN", 300),
            request_timeout: Duration::from_secs(num("REQUEST_TIMEOUT_SECS", 30)),
            body_limit_bytes: num("BODY_LIMIT_BYTES", 256 * 1024),
            llm_provider: var("LLM_PROVIDER").unwrap_or_else(|| "offline".into()),
            llm_api_key: var("LLM_API_KEY").or_else(|| var("OPENAI_API_KEY")),
            llm_base_url: var("LLM_BASE_URL").unwrap_or_else(|| {
                "https://generativelanguage.googleapis.com/v1beta/openai".into()
            }),
            llm_model: var("LLM_MODEL").unwrap_or_else(|| "gemini-2.5-flash-lite".into()),
            llm_interpretation_model: var("LLM_INTERPRETATION_MODEL")
                .or_else(|| var("LLM_MODEL"))
                .unwrap_or_else(|| "gemini-2.5-flash".into()),
            llm_interpretation_default: flag("LLM_INTERPRETATION", false),
            chain_enabled: flag("CHAIN_ENABLED", false),
            chain_rpc_url: var("CHAIN_RPC_URL").unwrap_or_else(|| "http://localhost:8545".into()),
            chain_id: num("CHAIN_ID", 31337),
            chain_private_key: var("CHAIN_PRIVATE_KEY"),
            chain_contract_address: var("CHAIN_CONTRACT_ADDRESS"),
            chain_deployment_file: var("CHAIN_DEPLOYMENT_FILE"),
            chain_explorer_url: var("CHAIN_EXPLORER_URL"),
            ipfs_enabled: flag("IPFS_ENABLED", false),
            ipfs_api_url: var("IPFS_API_URL").unwrap_or_else(|| "http://localhost:5001".into()),
            ipfs_gateway_url: var("IPFS_GATEWAY_URL")
                .unwrap_or_else(|| "http://localhost:8081".into()),
            anchor_interval: Duration::from_secs(num("ANCHOR_INTERVAL_SECS", 5)),
            worker_enabled: flag("ANCHOR_WORKER_ENABLED", true),
            env,
        })
    }

    /// Configuration for tests: in-memory SQLite, no Redis, everything local.
    pub fn for_tests() -> Self {
        let mut c = Config::from_env().expect("defaults are valid");
        c.env = AppEnv::Test;
        c.database_url = "sqlite::memory:".into();
        c.redis_url = None;
        c.chain_enabled = false;
        c.ipfs_enabled = false;
        c.worker_enabled = false;
        c.cookie_secure = false;
        c.allow_registration = true;
        c.llm_provider = "offline".into();
        c.rate_auth_per_min = 1000;
        c.rate_api_per_min = 10_000;
        c.rate_analyze_per_min = 1000;
        c
    }

    pub fn is_production(&self) -> bool {
        self.env == AppEnv::Production
    }
}
