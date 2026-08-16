use anyhow::{anyhow, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: usize,
    #[serde(default)]
    pub completion_tokens: usize,
    #[serde(default)]
    pub total_tokens: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LLMReply {
    pub content: String,
    pub reasoning_content: Option<String>,
    pub usage: Usage,
    pub latency_ms: u64,
}

#[derive(Debug, Clone)]
pub struct AIClient {
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub thinking: bool,
    pub reasoning_effort: String,
    pub timeout_seconds: u64,
    http: reqwest::Client,
}

impl AIClient {
    pub fn new(
        model: &str,
        base_url: &str,
        api_key: &str,
        thinking: bool,
        reasoning_effort: &str,
        timeout_seconds: u64,
    ) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if !api_key.trim().is_empty() {
            let auth_val = format!("Bearer {}", api_key.trim());
            if let Ok(hv) = HeaderValue::from_str(&auth_val) {
                headers.insert(AUTHORIZATION, hv);
            }
        }

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_seconds.max(10)))
            .default_headers(headers)
            .build()
            .unwrap_or_default();

        Self {
            model: model.trim().to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.trim().to_string(),
            thinking,
            reasoning_effort: reasoning_effort.trim().to_string(),
            timeout_seconds,
            http,
        }
    }

    pub async fn chat_completion(&self, messages: &[ChatMessage]) -> Result<LLMReply> {
        let endpoint = if self.base_url.ends_with("/chat/completions") {
            self.base_url.clone()
        } else if self.base_url.ends_with("/v1") {
            format!("{}/chat/completions", self.base_url)
        } else {
            format!("{}/v1/chat/completions", self.base_url)
        };

        let mut payload = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "stream": false,
        });

        // Add thinking / reasoning options if supported
        if self.thinking {
            payload["reasoning_effort"] = serde_json::json!(self.reasoning_effort);
        }

        let start = Instant::now();
        let resp = self.http.post(&endpoint).json(&payload).send().await?;
        let status = resp.status();
        let resp_text = resp.text().await?;
        let latency_ms = start.elapsed().as_millis() as u64;

        if !status.is_success() {
            return Err(anyhow!("LLM API HTTP {} error: {}", status, resp_text));
        }

        let resp_json: Value = serde_json::from_str(&resp_text)
            .map_err(|e| anyhow!("Failed to parse LLM response JSON: {} (raw: {})", e, resp_text))?;

        if let Some(err_obj) = resp_json.get("error") {
            let msg = err_obj.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown error");
            return Err(anyhow!("LLM provider error: {}", msg));
        }

        let choices = resp_json.get("choices").and_then(|v| v.as_array()).ok_or_else(|| {
            anyhow!("LLM response missing choices array")
        })?;

        let choice = choices.first().ok_or_else(|| anyhow!("Empty choices returned by LLM"))?;
        let message = choice.get("message").ok_or_else(|| anyhow!("Missing message in choice"))?;

        let content = message.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let reasoning_content = message.get("reasoning_content")
            .or_else(|| message.get("reasoning"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut usage = Usage::default();
        if let Some(u) = resp_json.get("usage") {
            usage.prompt_tokens = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            usage.completion_tokens = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            usage.total_tokens = u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        }

        Ok(LLMReply {
            content,
            reasoning_content,
            usage,
            latency_ms,
        })
    }
}
