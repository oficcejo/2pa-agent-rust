use crate::web::service::WebTradingService;
use crate::web::static_files::{get_static_asset, render_index};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Json, Response};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

pub type AppState = Arc<WebTradingService>;

#[derive(Debug, Deserialize)]
pub struct InstrumentsQuery {
    #[serde(default = "default_inst_type")]
    pub inst_type: String,
}
fn default_inst_type() -> String { "SPOT".to_string() }

#[derive(Debug, Deserialize)]
pub struct CandlesQuery {
    #[serde(default = "default_inst_id")]
    pub inst_id: String,
    #[serde(default = "default_timeframe")]
    pub timeframe: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}
fn default_inst_id() -> String { "BTC-USDT".to_string() }
fn default_timeframe() -> String { "15m".to_string() }
fn default_limit() -> usize { 120 }

#[derive(Debug, Deserialize)]
pub struct LimitQuery {
    #[serde(default = "default_limit_50")]
    pub limit: usize,
}
fn default_limit_50() -> usize { 50 }

#[derive(Debug, Deserialize)]
pub struct AnalyzeRequest {
    #[serde(default = "default_inst_id")]
    pub inst_id: String,
    #[serde(default = "default_timeframe")]
    pub timeframe: String,
    #[serde(default = "default_bar_count")]
    pub bar_count: usize,
    #[serde(default)]
    pub execute: bool,
}
fn default_bar_count() -> usize { 100 }

#[derive(Debug, Deserialize)]
pub struct AutomationRequest {
    pub enabled: bool,
    #[serde(default = "default_inst_id")]
    pub inst_id: String,
    #[serde(default = "default_timeframe")]
    pub timeframe: String,
    #[serde(default)]
    pub confirmation: String,
    pub session_preset: Option<String>,
    pub session_timezone: Option<String>,
    pub session_start: Option<String>,
    pub session_end: Option<String>,
    pub session_weekdays: Option<Vec<u32>>,
}

pub async fn handle_index() -> Html<String> {
    render_index()
}

pub async fn handle_static(AxumPath(path): AxumPath<String>) -> Response {
    get_static_asset(&path)
}

pub async fn handle_status(State(service): State<AppState>) -> Response {
    Json(service.status()).into_response()
}

pub async fn handle_instruments(
    State(service): State<AppState>,
    Query(query): Query<InstrumentsQuery>,
) -> Response {
    match service.instruments(&query.inst_type).await {
        Ok(data) => Json(serde_json::to_value(data).unwrap_or(Value::Array(Vec::new()))).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    }
}

pub async fn handle_candles(
    State(service): State<AppState>,
    Query(query): Query<CandlesQuery>,
) -> Response {
    match service.candles(&query.inst_id, &query.timeframe, query.limit).await {
        Ok(data) => Json(serde_json::to_value(data).unwrap_or(Value::Array(Vec::new()))).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    }
}

pub async fn handle_account(
    State(service): State<AppState>,
) -> Response {
    match service.account().await {
        Ok(data) => Json(data).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    }
}

pub async fn handle_get_decision_history(
    State(service): State<AppState>,
    Query(query): Query<LimitQuery>,
) -> Response {
    let records = service.decision_records(query.limit);
    Json(Value::Array(records)).into_response()
}

pub async fn handle_delete_decision_history(
    State(service): State<AppState>,
    AxumPath(record_id): AxumPath<String>,
) -> Response {
    if service.delete_decision_record(&record_id) {
        Json(serde_json::json!({ "deleted": true })).into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

pub async fn handle_get_trade_history(
    State(service): State<AppState>,
    Query(query): Query<LimitQuery>,
) -> Response {
    let records = service.trade_records(query.limit);
    Json(serde_json::to_value(records).unwrap_or(Value::Array(Vec::new()))).into_response()
}

pub async fn handle_delete_trade_history(
    State(service): State<AppState>,
    AxumPath(record_id): AxumPath<String>,
) -> Response {
    if service.delete_trade_record(&record_id) {
        Json(serde_json::json!({ "deleted": true })).into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

pub async fn handle_analyze(
    State(service): State<AppState>,
    Json(req): Json<AnalyzeRequest>,
) -> Response {
    match service.analyze(&req.inst_id, &req.timeframe, req.bar_count, req.execute).await {
        Ok(data) => Json(data).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

pub async fn handle_automation(
    State(service): State<AppState>,
    Json(req): Json<AutomationRequest>,
) -> Response {
    match service.set_automation(
        req.enabled,
        &req.inst_id,
        &req.timeframe,
        &req.confirmation,
        req.session_preset.as_deref(),
        req.session_timezone.as_deref(),
        req.session_start.as_deref(),
        req.session_end.as_deref(),
        req.session_weekdays.as_deref(),
    ) {
        Ok(data) => Json(data).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

pub async fn handle_get_config(
    State(service): State<AppState>,
) -> Response {
    Json(service.get_config()).into_response()
}

pub async fn handle_save_config(
    State(service): State<AppState>,
    Json(req): Json<crate::web::service::SaveEnvRequest>,
) -> Response {
    match service.save_env_config(&req) {
        Ok(data) => Json(data).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct ContractSpecsQuery {
    pub symbol: Option<String>,
}

pub async fn handle_contract_specs(
    State(service): State<AppState>,
    Query(query): Query<ContractSpecsQuery>,
) -> Response {
    match service.get_contract_specs(query.symbol.as_deref()).await {
        Ok(data) => Json(data).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}
