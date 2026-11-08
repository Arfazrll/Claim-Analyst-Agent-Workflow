//! Minimal client for the IPFS (Kubo) RPC API: pin encrypted ledger bundles and read
//! them back. Content addressing means the CID itself proves the bytes are unchanged.

use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct IpfsClient {
    api: String,
    pub gateway: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct AddResponse {
    #[serde(rename = "Hash")]
    hash: String,
}

impl IpfsClient {
    pub fn new(api: &str, gateway: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("http client");
        IpfsClient {
            api: api.trim_end_matches('/').to_string(),
            gateway: gateway.trim_end_matches('/').to_string(),
            http,
        }
    }

    pub async fn add_json(&self, name: &str, doc: &Value) -> Result<String, String> {
        let body = serde_json::to_vec(doc).map_err(|e| e.to_string())?;
        let part = reqwest::multipart::Part::bytes(body)
            .file_name(format!("{name}.json"))
            .mime_str("application/json")
            .map_err(|e| e.to_string())?;
        let form = reqwest::multipart::Form::new().part("file", part);
        let resp = self
            .http
            .post(format!("{}/api/v0/add?pin=true&cid-version=1", self.api))
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("ipfs add: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("ipfs add returned {}", resp.status()));
        }
        resp.json::<AddResponse>()
            .await
            .map(|r| r.hash)
            .map_err(|e| format!("ipfs add response: {e}"))
    }

    pub async fn cat_json(&self, cid: &str) -> Result<Value, String> {
        if !cid.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err("invalid CID".into());
        }
        let resp = self
            .http
            .post(format!("{}/api/v0/cat?arg={cid}", self.api))
            .send()
            .await
            .map_err(|e| format!("ipfs cat: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("ipfs cat returned {}", resp.status()));
        }
        resp.json::<Value>().await.map_err(|e| e.to_string())
    }

    pub async fn healthy(&self) -> bool {
        self.http
            .post(format!("{}/api/v0/id", self.api))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    pub fn gateway_url(&self, cid: &str) -> String {
        format!("{}/ipfs/{cid}", self.gateway)
    }
}
