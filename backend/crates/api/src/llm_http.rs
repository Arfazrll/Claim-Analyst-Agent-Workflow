//! OpenAI-compatible chat-completions client (Gemini, OpenAI, Groq, Together, local
//! Ollama, ...). Any failure returns `None`, so the deterministic pipeline stands.

use async_trait::async_trait;
use claim_engine::llm::{LlmClient, RATIONALE_SYSTEM};
use serde_json::{json, Value};

pub struct OpenAiCompatible {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    interpretation_model: String,
}

impl OpenAiCompatible {
    pub fn new(base_url: &str, api_key: &str, model: &str, interpretation_model: &str) -> Self {
        OpenAiCompatible {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("http client"),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            interpretation_model: interpretation_model.to_string(),
        }
    }

    async fn chat(
        &self,
        system: &str,
        user: &str,
        max_tokens: u32,
        timeout_s: u64,
        json_mode: bool,
        model: &str,
    ) -> Option<String> {
        let mut body = json!({
            "model": model, "temperature": 0, "max_tokens": max_tokens,
            "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
        });
        if json_mode {
            body["response_format"] = json!({"type": "json_object"});
        }
        let resp = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .timeout(std::time::Duration::from_secs(timeout_s))
            .json(&body)
            .send()
            .await;
        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::warn!(status = %r.status(), "LLM call failed; deterministic result stands");
                return None;
            }
            Err(e) => {
                tracing::warn!(error = %e, "LLM call failed; deterministic result stands");
                return None;
            }
        };
        let v: Value = resp.json().await.ok()?;
        v.pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
    }
}

#[async_trait]
impl LlmClient for OpenAiCompatible {
    fn provider(&self) -> &str {
        "openai_compatible"
    }
    async fn rationale(&self, prompt: &str) -> Option<String> {
        self.chat(RATIONALE_SYSTEM, prompt, 350, 20, false, &self.model)
            .await
    }
    async fn interpret(&self, system: &str, user: &str) -> Option<String> {
        self.chat(system, user, 6000, 120, true, &self.interpretation_model)
            .await
    }
}
