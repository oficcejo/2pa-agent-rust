use anyhow::{anyhow, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The three System One question primitive types supported by TypeSafe AI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuestionType {
    Choice,
    Score,
    Noul,
}

/// A strongly-typed question submitted to TypeSafe System One.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeSafeQuestion {
    #[serde(rename = "type")]
    pub question_type: QuestionType,
    pub instructions: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub criteria: Option<Value>,
}

impl TypeSafeQuestion {
    /// Create a yes/no proposition question (Noul).
    pub fn noul(instructions: impl Into<String>) -> Self {
        Self {
            question_type: QuestionType::Noul,
            instructions: Value::String(instructions.into()),
            criteria: None,
        }
    }

    /// Create a Noul question with explicit true/false semantic criteria.
    pub fn noul_with_criteria(
        instructions: impl Into<String>,
        true_meaning: impl Into<String>,
        false_meaning: impl Into<String>,
    ) -> Self {
        Self {
            question_type: QuestionType::Noul,
            instructions: Value::String(instructions.into()),
            criteria: Some(serde_json::json!({
                "true": true_meaning.into(),
                "false": false_meaning.into(),
            })),
        }
    }

    /// Create a discrete choice selection question (Choice).
    pub fn choice(
        instructions: impl Into<String>,
        criteria_options: HashMap<String, String>,
    ) -> Self {
        Self {
            question_type: QuestionType::Choice,
            instructions: Value::String(instructions.into()),
            criteria: Some(serde_json::to_value(criteria_options).unwrap_or(Value::Null)),
        }
    }

    /// Create an ordinal spectrum scoring question (Score).
    pub fn score(instructions: impl Into<String>, levels: Vec<String>) -> Self {
        Self {
            question_type: QuestionType::Score,
            instructions: Value::String(instructions.into()),
            criteria: Some(serde_json::to_value(levels).unwrap_or(Value::Null)),
        }
    }
}

/// Request payload for the TypeSafe evaluation endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeSafeRequest {
    pub state: Value,
    pub model: String,
    pub questions: HashMap<String, TypeSafeQuestion>,
}

/// An evaluated answer returned by TypeSafe AI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeSafeAnswer {
    #[serde(rename = "type")]
    pub answer_type: QuestionType,

    /// For Choice questions: the highest probability option.
    #[serde(default)]
    pub choice: Option<String>,

    /// For Noul questions: calibrated probability (0.0 - 1.0) that the proposition is true.
    #[serde(default)]
    pub noul: Option<f64>,

    /// For Score questions: expected score across defined levels.
    #[serde(default)]
    pub score: Option<f64>,

    /// Level descriptions index map for Score questions.
    #[serde(default)]
    pub legend: Option<HashMap<String, String>>,

    /// Discrete probability distribution across choices or score levels.
    #[serde(default)]
    pub probabilities: Option<HashMap<String, f64>>,

    /// Mathematical confidence metric (0.0 - 1.0) summarizing the peakedness of the distribution.
    #[serde(default)]
    pub confidence: Option<f64>,
}

impl TypeSafeAnswer {
    /// Return the confidence value, or infer from Noul probability distance to 0.5 if not present.
    pub fn effective_confidence(&self) -> f64 {
        if let Some(conf) = self.confidence {
            conf
        } else if let Some(p) = self.noul {
            // For Noul: 0.5 is maximum uncertainty (conf = 0.0), 1.0 or 0.0 is maximum certainty (conf = 1.0)
            (p - 0.5).abs() * 2.0
        } else {
            0.0
        }
    }
}

/// Token usage reported by TypeSafe AI.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TypeSafeUsage {
    #[serde(default)]
    pub input_tokens: usize,
    #[serde(default)]
    pub output_tokens: usize,
}

/// Response payload from TypeSafe evaluation endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeSafeResponse {
    pub model: String,
    pub answers: HashMap<String, TypeSafeAnswer>,
    #[serde(default)]
    pub usage: TypeSafeUsage,
    #[serde(default)]
    pub latency_ms: u64,
}

/// Native client for communicating with TypeSafe AI System One API.
#[derive(Debug, Clone)]
pub struct TypeSafeClient {
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub timeout_seconds: u64,
    http: reqwest::Client,
}

impl TypeSafeClient {
    pub fn new(model: &str, base_url: &str, api_key: &str, timeout_seconds: u64) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if !api_key.trim().is_empty() {
            let auth_val = format!("Bearer {}", api_key.trim());
            if let Ok(hv) = HeaderValue::from_str(&auth_val) {
                headers.insert(AUTHORIZATION, hv);
            }
        }

        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_seconds.max(5)))
            .default_headers(headers);

        if let Ok(proxy_url) = std::env::var("LLM_PROXY")
            .or_else(|_| std::env::var("HTTPS_PROXY"))
            .or_else(|_| std::env::var("HTTP_PROXY"))
            .or_else(|_| std::env::var("ALL_PROXY"))
            .or_else(|_| std::env::var("https_proxy"))
            .or_else(|_| std::env::var("http_proxy"))
            .or_else(|_| std::env::var("all_proxy"))
        {
            if let Ok(p) = reqwest::Proxy::all(&proxy_url) {
                builder = builder.proxy(p);
            }
        }

        let http = builder.build().unwrap_or_default();

        Self {
            model: if model.trim().is_empty() {
                "jev-latest".to_string()
            } else {
                model.trim().to_string()
            },
            base_url: if base_url.trim().is_empty() {
                "https://api.typesafe.ai/v1".to_string()
            } else {
                base_url.trim_end_matches('/').to_string()
            },
            api_key: api_key.trim().to_string(),
            timeout_seconds: timeout_seconds.max(5),
            http,
        }
    }

    /// Check whether this client has a valid, non-empty API key configured.
    pub fn has_api_key(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    /// Evaluate a state against a map of typed questions using TypeSafe's System One model.
    pub async fn evaluate(
        &self,
        state: &Value,
        questions: &HashMap<String, TypeSafeQuestion>,
    ) -> Result<TypeSafeResponse> {
        if !self.has_api_key() {
            return Err(anyhow!("TypeSafe API key is missing or empty"));
        }

        let endpoint = if self.base_url.ends_with("/systemone") {
            self.base_url.clone()
        } else {
            format!("{}/systemone", self.base_url)
        };

        let payload = TypeSafeRequest {
            state: state.clone(),
            model: self.model.clone(),
            questions: questions.clone(),
        };

        let start = Instant::now();
        let resp = self.http.post(&endpoint).json(&payload).send().await?;
        let status = resp.status();
        let resp_text = resp.text().await?;
        let latency_ms = start.elapsed().as_millis() as u64;

        if !status.is_success() {
            return Err(anyhow!(
                "TypeSafe API HTTP {} error: {}",
                status,
                resp_text
            ));
        }

        let mut parsed: TypeSafeResponse = serde_json::from_str(&resp_text).map_err(|e| {
            anyhow!(
                "Failed to parse TypeSafe response JSON: {} (raw: {})",
                e,
                resp_text
            )
        })?;

        parsed.latency_ms = latency_ms;
        Ok(parsed)
    }
}
