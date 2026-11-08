//! EVM anchoring through the `ClaimLedger` contract (see `blockchain/src/ClaimLedger.sol`).
//!
//! Works with any EVM JSON-RPC endpoint: a local Anvil node (free, default in
//! docker-compose), or a public testnet such as Sepolia / Polygon Amoy.

use std::str::FromStr;

use alloy::network::EthereumWallet;
use alloy::primitives::{Address, FixedBytes};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use alloy::sol;
use serde::Deserialize;

sol! {
    #[sol(rpc)]
    interface IClaimLedger {
        function anchor(bytes32 recordId, bytes32 dataHash, uint8 kind, string calldata cid) external returns (uint256 index);
        function isAnchored(bytes32 dataHash) external view returns (bool);
        function entryCount(bytes32 recordId) external view returns (uint256);
        function writers(address account) external view returns (bool);
    }
}

#[derive(Clone)]
pub struct ChainClient {
    rpc_url: url::Url,
    signer: PrivateKeySigner,
    pub address: Address,
    pub chain_id: u64,
    pub explorer_url: Option<String>,
}

impl std::fmt::Debug for ChainClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainClient")
            .field("address", &self.address)
            .field("chain_id", &self.chain_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Deserialize)]
struct DeploymentFile {
    address: String,
}

fn b32(hex_str: &str) -> Result<FixedBytes<32>, String> {
    let raw = hex::decode(hex_str.trim_start_matches("0x")).map_err(|e| e.to_string())?;
    let arr: [u8; 32] = raw
        .try_into()
        .map_err(|_| "expected 32 bytes".to_string())?;
    Ok(FixedBytes(arr))
}

impl ChainClient {
    pub fn new(
        rpc_url: &str,
        private_key: &str,
        address: &str,
        chain_id: u64,
        explorer_url: Option<String>,
    ) -> Result<Self, String> {
        let signer = PrivateKeySigner::from_str(private_key.trim())
            .map_err(|e| format!("invalid CHAIN_PRIVATE_KEY: {e}"))?;
        let address = Address::from_str(address.trim())
            .map_err(|e| format!("invalid contract address: {e}"))?;
        let rpc_url =
            url::Url::parse(rpc_url).map_err(|e| format!("invalid CHAIN_RPC_URL: {e}"))?;
        Ok(ChainClient {
            rpc_url,
            signer: signer.with_chain_id(Some(chain_id)),
            address,
            chain_id,
            explorer_url,
        })
    }

    /// Contract address from config, or from the file written by the deploy job.
    pub fn resolve_address(
        explicit: Option<&str>,
        deployment_file: Option<&str>,
    ) -> Option<String> {
        if let Some(a) = explicit {
            return Some(a.to_string());
        }
        let path = deployment_file?;
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str::<DeploymentFile>(&raw)
            .ok()
            .map(|d| d.address)
    }

    pub fn signer_address(&self) -> Address {
        self.signer.address()
    }

    /// Send `anchor(...)` and wait for the receipt. Returns (tx hash, block number).
    pub async fn anchor(
        &self,
        record_id: &str,
        data_hash: &str,
        kind: u8,
        cid: &str,
    ) -> Result<(String, Option<u64>), String> {
        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(self.signer.clone()))
            .connect_http(self.rpc_url.clone());
        let contract = IClaimLedger::new(self.address, &provider);
        let pending = contract
            .anchor(b32(record_id)?, b32(data_hash)?, kind, cid.to_string())
            .send()
            .await
            .map_err(|e| format!("send failed: {e}"))?;
        let receipt =
            tokio::time::timeout(std::time::Duration::from_secs(90), pending.get_receipt())
                .await
                .map_err(|_| "timed out waiting for receipt".to_string())?
                .map_err(|e| format!("receipt failed: {e}"))?;
        if !receipt.status() {
            return Err(format!("transaction {} reverted", receipt.transaction_hash));
        }
        Ok((
            format!("{:#x}", receipt.transaction_hash),
            receipt.block_number,
        ))
    }

    pub async fn is_anchored(&self, data_hash: &str) -> Result<bool, String> {
        let provider = ProviderBuilder::new().connect_http(self.rpc_url.clone());
        let contract = IClaimLedger::new(self.address, &provider);
        contract
            .isAnchored(b32(data_hash)?)
            .call()
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn status(&self) -> Result<(u64, bool), String> {
        let provider = ProviderBuilder::new().connect_http(self.rpc_url.clone());
        let block = provider
            .get_block_number()
            .await
            .map_err(|e| e.to_string())?;
        let contract = IClaimLedger::new(self.address, &provider);
        let writer = contract
            .writers(self.signer.address())
            .call()
            .await
            .map_err(|e| e.to_string())?;
        Ok((block, writer))
    }

    pub fn tx_url(&self, tx: &str) -> Option<String> {
        self.explorer_url
            .as_ref()
            .map(|base| format!("{}/tx/{tx}", base.trim_end_matches('/')))
    }
}
