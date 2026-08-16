use anyhow::{anyhow, Result};
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE, USER_AGENT};
use serde_json::Value;
use sha2::Sha256;
use std::time::Duration;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub struct OKXCredentials {
    pub api_key: String,
    pub secret_key: String,
    pub passphrase: String,
}

impl OKXCredentials {
    pub fn new(api_key: &str, secret_key: &str, passphrase: &str) -> Self {
        Self {
            api_key: api_key.trim().to_string(),
            secret_key: secret_key.trim().to_string(),
            passphrase: passphrase.trim().to_string(),
        }
    }

    pub fn is_valid(&self) -> bool {
        !self.api_key.is_empty() && !self.secret_key.is_empty() && !self.passphrase.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct OKXClient {
    base_url: String,
    credentials: Option<OKXCredentials>,
    demo_trading: bool,
    http: reqwest::Client,
}

impl OKXClient {
    pub fn new(
        base_url: &str,
        credentials: Option<OKXCredentials>,
        demo_trading: bool,
        timeout_seconds: u64,
    ) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(USER_AGENT, HeaderValue::from_static("PA-Agent-OKX-Rust/1.0"));

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_seconds.max(5)))
            .default_headers(headers)
            .build()
            .unwrap_or_default();

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            credentials,
            demo_trading,
            http,
        }
    }

    fn timestamp_iso() -> String {
        let now = Utc::now();
        format!("{}.{:03}Z", now.format("%Y-%m-%dT%H:%M:%S"), now.timestamp_subsec_millis())
    }

    fn sign(&self, timestamp: &str, method: &str, request_path: &str, body: &str) -> Result<String> {
        let creds = self.credentials.as_ref().ok_or_else(|| {
            anyhow!("OKX credentials are not configured")
        })?;

        let prehash = format!("{}{}{}{}", timestamp, method.to_uppercase(), request_path, body);
        let mut mac = HmacSha256::new_from_slice(creds.secret_key.as_bytes())
            .map_err(|e| anyhow!("HMAC init error: {}", e))?;
        mac.update(prehash.as_bytes());
        let result = mac.finalize().into_bytes();
        Ok(base64::engine::general_purpose::STANDARD.encode(result))
    }

    pub async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        query_params: Option<&[(&str, &str)]>,
        body_value: Option<&Value>,
        auth: bool,
    ) -> Result<Vec<Value>> {
        let query_string = query_params
            .map(|params| {
                let s = params
                    .iter()
                    .filter(|(_, v)| !v.is_empty())
                    .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
                    .collect::<Vec<_>>()
                    .join("&");
                if s.is_empty() { String::new() } else { format!("?{}", s) }
            })
            .unwrap_or_default();

        let request_path = format!("{}{}", path, query_string);
        let url = format!("{}{}", self.base_url, request_path);

        let body_str = if let Some(b) = body_value {
            serde_json::to_string(b)?
        } else {
            String::new()
        };

        let mut req_builder = self.http.request(method.clone(), &url);

        if auth {
            let creds = self.credentials.as_ref().ok_or_else(|| {
                anyhow!("OKX credentials are not configured")
            })?;
            let timestamp = Self::timestamp_iso();
            let sign = self.sign(&timestamp, method.as_str(), &request_path, &body_str)?;

            req_builder = req_builder
                .header("OK-ACCESS-KEY", &creds.api_key)
                .header("OK-ACCESS-SIGN", &sign)
                .header("OK-ACCESS-TIMESTAMP", &timestamp)
                .header("OK-ACCESS-PASSPHRASE", &creds.passphrase)
                .header(CONTENT_TYPE, "application/json");

            if self.demo_trading {
                req_builder = req_builder.header("x-simulated-trading", "1");
            }
        }

        if !body_str.is_empty() {
            req_builder = req_builder.body(body_str);
        }

        let resp = req_builder.send().await?;
        let status = resp.status();
        let resp_text = resp.text().await?;

        if !status.is_success() {
            return Err(anyhow!("OKX HTTP {} error: {}", status, resp_text));
        }

        let resp_json: Value = serde_json::from_str(&resp_text)
            .map_err(|e| anyhow!("Failed to parse OKX JSON: {} (raw: {})", e, resp_text))?;

        let code = resp_json.get("code").and_then(|v| v.as_str()).unwrap_or("");
        if code != "0" {
            let msg = resp_json.get("msg").and_then(|v| v.as_str()).unwrap_or("Unknown OKX error");
            return Err(anyhow!("OKX API error [{}]: {}", code, msg));
        }

        let data = resp_json.get("data").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        Ok(data)
    }

    pub async fn get_instruments(&self, inst_type: &str, inst_id: Option<&str>) -> Result<Vec<Value>> {
        let mut params = vec![("instType", inst_type)];
        if let Some(id) = inst_id {
            if !id.is_empty() {
                params.push(("instId", id));
            }
        }
        self.request(reqwest::Method::GET, "/api/v5/public/instruments", Some(&params), None, false).await
    }

    pub async fn get_tickers(&self, inst_type: &str) -> Result<Vec<Value>> {
        let params = [("instType", inst_type)];
        self.request(reqwest::Method::GET, "/api/v5/market/tickers", Some(&params), None, false).await
    }

    pub async fn get_ticker(&self, inst_id: &str) -> Result<Value> {
        let params = [("instId", inst_id)];
        let data = self.request(reqwest::Method::GET, "/api/v5/market/ticker", Some(&params), None, false).await?;
        data.into_iter().next().ok_or_else(|| anyhow!("Ticker not found for {}", inst_id))
    }

    pub async fn get_candles(
        &self,
        inst_id: &str,
        bar: &str,
        limit: usize,
    ) -> Result<Vec<Vec<String>>> {
        let limit_str = limit.min(300).to_string();
        let params = [
            ("instId", inst_id),
            ("bar", bar),
            ("limit", &limit_str),
        ];
        let data = self.request(reqwest::Method::GET, "/api/v5/market/candles", Some(&params), None, false).await?;

        let mut candles = Vec::with_capacity(data.len());
        for row in data {
            if let Some(arr) = row.as_array() {
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| v.as_str().unwrap_or_default().to_string())
                    .collect();
                candles.push(items);
            }
        }
        Ok(candles)
    }

    pub async fn get_account_balance(&self) -> Result<Vec<Value>> {
        self.request(reqwest::Method::GET, "/api/v5/account/balance", None, None, true).await
    }

    pub async fn get_positions(&self, inst_id: Option<&str>) -> Result<Vec<Value>> {
        let mut params = Vec::new();
        if let Some(id) = inst_id {
            if !id.is_empty() {
                params.push(("instId", id));
            }
        }
        let p = if params.is_empty() { None } else { Some(params.as_slice()) };
        self.request(reqwest::Method::GET, "/api/v5/account/positions", p, None, true).await
    }

    pub async fn get_pending_orders(&self, inst_id: Option<&str>) -> Result<Vec<Value>> {
        let mut params = Vec::new();
        if let Some(id) = inst_id {
            if !id.is_empty() {
                params.push(("instId", id));
            }
        }
        let p = if params.is_empty() { None } else { Some(params.as_slice()) };
        self.request(reqwest::Method::GET, "/api/v5/trade/orders-pending", p, None, true).await
    }

    pub async fn get_pending_algo_orders(&self, inst_id: Option<&str>, order_type: &str) -> Result<Vec<Value>> {
        let mut params = vec![("ordType", order_type)];
        if let Some(id) = inst_id {
            if !id.is_empty() {
                params.push(("instId", id));
            }
        }
        self.request(reqwest::Method::GET, "/api/v5/trade/orders-algo-pending", Some(&params), None, true).await
    }

    pub async fn get_order(&self, inst_id: &str, ord_id: Option<&str>, cl_ord_id: Option<&str>) -> Result<Value> {
        let mut params = vec![("instId", inst_id)];
        if let Some(id) = ord_id {
            if !id.is_empty() { params.push(("ordId", id)); }
        }
        if let Some(cid) = cl_ord_id {
            if !cid.is_empty() { params.push(("clOrdId", cid)); }
        }
        let data = self.request(reqwest::Method::GET, "/api/v5/trade/order", Some(&params), None, true).await?;
        data.into_iter().next().ok_or_else(|| anyhow!("Order not found"))
    }

    pub async fn get_algo_order(&self, algo_id: Option<&str>, algo_cl_ord_id: Option<&str>) -> Result<Value> {
        let mut params = Vec::new();
        if let Some(id) = algo_id {
            if !id.is_empty() { params.push(("algoId", id)); }
        }
        if let Some(cid) = algo_cl_ord_id {
            if !cid.is_empty() { params.push(("algoClOrdId", cid)); }
        }
        let data = self.request(reqwest::Method::GET, "/api/v5/trade/order-algo", Some(&params), None, true).await?;
        data.into_iter().next().ok_or_else(|| anyhow!("Algo order not found"))
    }

    pub async fn set_leverage(&self, inst_id: &str, leverage: &str, margin_mode: &str) -> Result<Value> {
        let payload = serde_json::json!({
            "instId": inst_id,
            "lever": leverage,
            "mgnMode": margin_mode,
        });
        let data = self.request(reqwest::Method::POST, "/api/v5/account/set-leverage", None, Some(&payload), true).await?;
        Ok(data.into_iter().next().unwrap_or_default())
    }

    pub async fn place_order(&self, payload: &Value) -> Result<Value> {
        let data = self.request(reqwest::Method::POST, "/api/v5/trade/order", None, Some(payload), true).await?;
        let result = data.into_iter().next().unwrap_or_default();
        let s_code = result.get("sCode").and_then(|v| v.as_str()).unwrap_or("0");
        if s_code != "0" {
            let s_msg = result.get("sMsg").and_then(|v| v.as_str()).unwrap_or("OKX rejected order");
            return Err(anyhow!("OKX order failed [{}]: {}", s_code, s_msg));
        }
        Ok(result)
    }

    pub async fn place_algo_order(&self, payload: &Value) -> Result<Value> {
        let data = self.request(reqwest::Method::POST, "/api/v5/trade/order-algo", None, Some(payload), true).await?;
        let result = data.into_iter().next().unwrap_or_default();
        let s_code = result.get("sCode").and_then(|v| v.as_str()).unwrap_or("0");
        if s_code != "0" {
            let s_msg = result.get("sMsg").and_then(|v| v.as_str()).unwrap_or("OKX rejected algo order");
            return Err(anyhow!("OKX algo order failed [{}]: {}", s_code, s_msg));
        }
        Ok(result)
    }

    pub async fn cancel_order(&self, inst_id: &str, ord_id: Option<&str>, cl_ord_id: Option<&str>) -> Result<Value> {
        let mut payload = serde_json::json!({ "instId": inst_id });
        if let Some(id) = ord_id {
            if !id.is_empty() { payload["ordId"] = serde_json::json!(id); }
        }
        if let Some(cid) = cl_ord_id {
            if !cid.is_empty() { payload["clOrdId"] = serde_json::json!(cid); }
        }
        let data = self.request(reqwest::Method::POST, "/api/v5/trade/cancel-order", None, Some(&payload), true).await?;
        let result = data.into_iter().next().unwrap_or_default();
        let s_code = result.get("sCode").and_then(|v| v.as_str()).unwrap_or("0");
        if s_code != "0" {
            let s_msg = result.get("sMsg").and_then(|v| v.as_str()).unwrap_or("Cancel rejected");
            return Err(anyhow!("OKX cancel failed [{}]: {}", s_code, s_msg));
        }
        Ok(result)
    }

    pub async fn cancel_algo_order(&self, inst_id: &str, algo_id: &str) -> Result<Value> {
        let payload = serde_json::json!([{
            "instId": inst_id,
            "algoId": algo_id,
        }]);
        let data = self.request(reqwest::Method::POST, "/api/v5/trade/cancel-algos", None, Some(&payload), true).await?;
        let result = data.into_iter().next().unwrap_or_default();
        let s_code = result.get("sCode").and_then(|v| v.as_str()).unwrap_or("0");
        if s_code != "0" {
            let s_msg = result.get("sMsg").and_then(|v| v.as_str()).unwrap_or("Cancel algo rejected");
            return Err(anyhow!("OKX cancel algo failed [{}]: {}", s_code, s_msg));
        }
        Ok(result)
    }
}
