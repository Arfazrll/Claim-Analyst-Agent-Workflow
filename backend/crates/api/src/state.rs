use std::sync::Arc;

use claim_engine::{Engine, LlmClient, OfflineLlm};

use crate::cache::Cache;
use crate::config::Config;
use crate::crypto::Cipher;
use crate::db::Db;
use crate::ledger::chain::ChainClient;
use crate::ledger::ipfs::IpfsClient;
use crate::ledger::Ledger;
use crate::llm_http::OpenAiCompatible;
use crate::metrics::Metrics;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub db: Db,
    pub cache: Cache,
    pub engine: Engine,
    pub llm: Arc<dyn LlmClient>,
    pub ledger: Ledger,
    pub cipher: Cipher,
    pub metrics: Arc<Metrics>,
    pub instance_id: String,
    pub started_at: std::time::Instant,
}

impl AppState {
    pub async fn build(cfg: Config) -> Result<AppState, String> {
        let db = Db::connect(&cfg.database_url)
            .await
            .map_err(|e| format!("database: {e}"))?;
        let cache = Cache::connect(cfg.redis_url.as_deref()).await;
        let cipher = Cipher::new(cfg.data_key);

        let llm: Arc<dyn LlmClient> = match (cfg.llm_provider.as_str(), &cfg.llm_api_key) {
            ("openai_compatible", Some(key)) => Arc::new(OpenAiCompatible::new(
                &cfg.llm_base_url,
                key,
                &cfg.llm_model,
                &cfg.llm_interpretation_model,
            )),
            ("openai_compatible", None) => {
                tracing::warn!(
                    "LLM_PROVIDER=openai_compatible but no LLM_API_KEY; running offline"
                );
                Arc::new(OfflineLlm)
            }
            _ => Arc::new(OfflineLlm),
        };

        let chain = if cfg.chain_enabled {
            let address = ChainClient::resolve_address(cfg.chain_contract_address.as_deref(), cfg.chain_deployment_file.as_deref())
                .ok_or("CHAIN_ENABLED=true but no CHAIN_CONTRACT_ADDRESS or readable CHAIN_DEPLOYMENT_FILE")?;
            let key = cfg
                .chain_private_key
                .as_deref()
                .ok_or("CHAIN_ENABLED=true but CHAIN_PRIVATE_KEY is not set")?;
            let client = ChainClient::new(
                &cfg.chain_rpc_url,
                key,
                &address,
                cfg.chain_id,
                cfg.chain_explorer_url.clone(),
            )?;
            tracing::info!(contract = %client.address, signer = %client.signer_address(), chain_id = cfg.chain_id, "blockchain anchoring enabled");
            Some(client)
        } else {
            None
        };
        let ipfs = cfg
            .ipfs_enabled
            .then(|| IpfsClient::new(&cfg.ipfs_api_url, &cfg.ipfs_gateway_url));
        let ledger = Ledger {
            db: db.clone(),
            cipher: cipher.clone(),
            chain,
            ipfs,
        };

        let engine_cfg = claim_engine::EngineConfig {
            interpret_by_default: cfg.llm_interpretation_default,
            ..Default::default()
        };
        let retriever =
            claim_engine::HybridRetriever::from_jsonl(claim_engine::BUNDLED_POLICY_JSONL)
                .map_err(|e| e.to_string())?;
        let engine = Engine::new(retriever, engine_cfg);

        let instance_id = format!(
            "{}-{}",
            std::env::var("HOSTNAME").unwrap_or_else(|_| "local".into()),
            &uuid::Uuid::new_v4().to_string()[..8]
        );
        Ok(AppState {
            cfg: Arc::new(cfg),
            db,
            cache,
            engine,
            llm,
            ledger,
            cipher,
            metrics: Arc::new(Metrics::default()),
            instance_id,
            started_at: std::time::Instant::now(),
        })
    }
}
