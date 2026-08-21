use crate::config::paths::{records_dir, settings_json_path};
use crate::config::settings::Settings;
use crate::data::base::{KlineBar, PositionContext};
use crate::data::snapshot::{build_analysis_frame, build_live_frame, INDICATOR_WARMUP_BARS};
use crate::okx::client::{OKXClient, OKXCredentials};
use crate::okx::trading::{AuditEntry, OKXTradeExecutor, BROKER_TAG};
use crate::orchestrator::two_stage::TwoStageOrchestrator;
use crate::records::history::{delete_record, list_record_paths, load_record};
use crate::util::mask::mask_secret;
use crate::web::sessions::{build_trading_session, TradingSession};
use anyhow::{anyhow, Result};
use chrono::{Timelike, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveEnvRequest {
    #[serde(default)]
    pub llm_api_key: String,
    #[serde(default = "default_llm_base_url")]
    pub llm_base_url: String,
    #[serde(default = "default_llm_model")]
    pub llm_model: String,
    #[serde(default)]
    pub llm_thinking: bool,

    #[serde(default = "default_trading_system")]
    pub trading_system: String,

    #[serde(default)]
    pub okx_api_key: String,
    #[serde(default)]
    pub okx_secret_key: String,
    #[serde(default)]
    pub okx_passphrase: String,
    #[serde(default = "default_okx_base_url")]
    pub okx_base_url: String,
    #[serde(default = "default_true")]
    pub okx_demo_trading: bool,

    #[serde(default = "default_order_size")]
    pub okx_default_order_size: f64,
    #[serde(default = "default_leverage")]
    pub okx_default_leverage: f64,
    #[serde(default = "default_trade_mode")]
    pub okx_trade_mode: String,
    #[serde(default = "default_position_mode")]
    pub okx_position_mode: String,
}

fn default_llm_base_url() -> String { "https://api.deepseek.com".to_string() }
fn default_llm_model() -> String { "deepseek-v4-flash".to_string() }
fn default_trading_system() -> String { "2pa".to_string() }
fn default_okx_base_url() -> String { "https://www.okx.com".to_string() }
fn default_true() -> bool { true }
fn default_order_size() -> f64 { 1.0 }
fn default_leverage() -> f64 { 3.0 }
fn default_trade_mode() -> String { "cross".to_string() }
fn default_position_mode() -> String { "net".to_string() }

pub struct WebTradingService {
    pub settings: Arc<RwLock<Settings>>,
    pub okx_client: Arc<RwLock<OKXClient>>,
    pub executor: Arc<RwLock<OKXTradeExecutor>>,
    pub orchestrator: Arc<RwLock<TwoStageOrchestrator>>,
    pub current_trading_system: Arc<RwLock<String>>,
    pub automation_enabled: Arc<RwLock<bool>>,
    pub automation_symbol: Arc<RwLock<String>>,
    pub automation_timeframe: Arc<RwLock<String>>,
    pub automation_session: Arc<RwLock<TradingSession>>,
    pub latest_analysis: Arc<RwLock<Option<Value>>>,
    pub last_closed_ts: Arc<RwLock<HashMap<(String, String), i64>>>,
    pub equity_history: Arc<RwLock<Vec<Value>>>,
}

impl WebTradingService {
    pub fn new(settings: Settings) -> Self {
        let creds = if settings.is_okx_configured() {
            Some(OKXCredentials::new(
                &settings.okx.api_key,
                &settings.okx.secret_key,
                &settings.okx.passphrase,
            ))
        } else {
            None
        };

        let okx_client = OKXClient::new(
            &settings.okx.base_url,
            creds,
            settings.okx.demo_trading,
            15,
        );

        let audit_path = Some(records_dir().join("trade_audit.jsonl"));
        let executor = OKXTradeExecutor::new(
            okx_client.clone(),
            settings.okx.default_order_size,
            &settings.okx.trade_mode,
            &settings.okx.position_mode,
            settings.okx.default_leverage,
            settings.okx.block_new_entries_when_position_open,
            settings.general.decision_confidence_threshold,
            settings.okx.max_signal_age_seconds,
            settings.okx.max_pending_bars,
            audit_path,
        );

        let orchestrator = TwoStageOrchestrator::new(
            settings.clone(),
            records_dir(),
        );

        let session = build_trading_session(
            &settings.okx.automation_session_preset,
            &settings.okx.automation_session_timezone,
            &settings.okx.automation_session_start,
            &settings.okx.automation_session_end,
            Some(&settings.okx.automation_session_weekdays),
        );

        let initial_system = settings.general.trading_system.clone();

        Self {
            settings: Arc::new(RwLock::new(settings)),
            okx_client: Arc::new(RwLock::new(okx_client)),
            executor: Arc::new(RwLock::new(executor)),
            orchestrator: Arc::new(RwLock::new(orchestrator)),
            current_trading_system: Arc::new(RwLock::new(initial_system)),
            automation_enabled: Arc::new(RwLock::new(false)),
            automation_symbol: Arc::new(RwLock::new("BTC-USDT".to_string())),
            automation_timeframe: Arc::new(RwLock::new("15m".to_string())),
            automation_session: Arc::new(RwLock::new(session)),
            latest_analysis: Arc::new(RwLock::new(None)),
            last_closed_ts: Arc::new(RwLock::new(HashMap::new())),
            equity_history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn status(&self) -> Value {
        let settings = self.settings.read();
        let session = self.automation_session.read();
        let auto_enabled = *self.automation_enabled.read();
        let symbol = self.automation_symbol.read().clone();
        let timeframe = self.automation_timeframe.read().clone();
        let latest = self.latest_analysis.read().clone();
        let trading_system = self.current_trading_system.read().clone();

        serde_json::json!({
            "ok": true,
            "mode": if settings.okx.demo_trading { "demo" } else { "live" },
            "has_env_file": std::path::Path::new(".env").exists(),
            "is_ai_configured": settings.is_provider_configured(),
            "credentials_configured": settings.is_okx_configured(),
            "auto_trading_enabled": auto_enabled,
            "live_execution_unlocked": settings.okx.demo_trading || settings.okx.live_trading_acknowledged,
            "can_execute": auto_enabled && settings.is_okx_configured(),
            "broker_tag": BROKER_TAG,
            "symbol": symbol,
            "timeframe": timeframe,
            "trading_system": trading_system,
            "available_trading_systems": [
                {
                    "id": "2pa",
                    "name": "2PA 价格行为系统 (Al Brooks)",
                    "description": "基于经典价格行为学八态周期、EMA20 与二元决策树"
                },
                {
                    "id": "dog_walking",
                    "name": "🐕 遛狗系统 (SMA 14/170 均线回归)",
                    "description": "基于 14 狗绳与 170 主人均线偏离力学与均值回归"
                }
            ],
            "confidence_threshold": settings.general.decision_confidence_threshold,
            "default_order_size": settings.okx.default_order_size,
            "default_leverage": settings.okx.default_leverage,
            "trade_mode": settings.okx.trade_mode,
            "position_mode": settings.okx.position_mode,
            "block_new_entries_when_position_open": settings.okx.block_new_entries_when_position_open,
            "max_pending_bars": settings.okx.max_pending_bars,
            "automation_session": session.as_dict(Some(Utc::now())),
            "automation_session_presets": crate::web::sessions::session_preset_options(),
            "latest": latest,
        })
    }

    pub fn get_config(&self) -> Value {
        let settings = self.settings.read();
        let cur_sys = self.current_trading_system.read().clone();
        serde_json::json!({
            "has_env_file": std::path::Path::new(".env").exists(),
            "is_configured": settings.is_provider_configured() && settings.is_okx_configured(),
            "is_ai_configured": settings.is_provider_configured(),
            "is_okx_configured": settings.is_okx_configured(),
            "llm_api_key": mask_secret(&settings.provider.api_key),
            "llm_base_url": settings.provider.base_url,
            "llm_model": settings.provider.model,
            "llm_thinking": settings.provider.thinking,
            "trading_system": cur_sys,
            "okx_api_key": mask_secret(&settings.okx.api_key),
            "okx_secret_key": mask_secret(&settings.okx.secret_key),
            "okx_passphrase": mask_secret(&settings.okx.passphrase),
            "okx_base_url": settings.okx.base_url,
            "okx_demo_trading": settings.okx.demo_trading,
            "okx_default_order_size": settings.okx.default_order_size,
            "okx_default_leverage": settings.okx.default_leverage,
            "okx_trade_mode": settings.okx.trade_mode,
            "okx_position_mode": settings.okx.position_mode,
        })
    }

    pub fn save_env_config(&self, req: &SaveEnvRequest) -> Result<Value> {
        let content = format!(
            r#"# =============================================================================
# OKX 2PA Agent 运行时环境变量配置文件 (由系统向导自动生成)
# =============================================================================

# ------------------------------ 大语言模型配置 ------------------------------
LLM_API_KEY={}
LLM_BASE_URL={}
LLM_MODEL={}
LLM_THINKING={}
LLM_REASONING_EFFORT=high
LLM_CONTEXT_WINDOW=128000
LLM_STAGE_TIMEOUT_SECONDS=240

# ------------------------------ 交易系统选择 ------------------------------
TRADING_SYSTEM={}

# ------------------------------ OKX API 凭证 ------------------------------
OKX_API_KEY={}
OKX_SECRET_KEY={}
OKX_PASSPHRASE={}
OKX_BASE_URL={}

# ------------------------------ 交易环境与开关 ------------------------------
OKX_DEMO_TRADING={}
OKX_AUTO_TRADING_ENABLED=false
OKX_LIVE_TRADING_ACKNOWLEDGED={}
OKX_ENABLE_LIVE_TRADING=YES

# ------------------------------ 订单与风控 ------------------------------
OKX_DEFAULT_ORDER_SIZE={}
OKX_DEFAULT_LEVERAGE={}
OKX_TRADE_MODE={}
OKX_POSITION_MODE={}
OKX_BLOCK_NEW_ENTRIES_WHEN_POSITION_OPEN=true
OKX_MAX_SIGNAL_AGE_SECONDS=120
OKX_MAX_PENDING_BARS=3

# ------------------------------ 交易时段 ------------------------------
OKX_AUTOMATION_SESSION_PRESET=always
OKX_AUTOMATION_SESSION_TIMEZONE=UTC
"#,
            req.llm_api_key.trim(),
            req.llm_base_url.trim(),
            req.llm_model.trim(),
            req.llm_thinking,
            req.trading_system.trim(),
            req.okx_api_key.trim(),
            req.okx_secret_key.trim(),
            req.okx_passphrase.trim(),
            req.okx_base_url.trim(),
            req.okx_demo_trading,
            !req.okx_demo_trading,
            req.okx_default_order_size,
            req.okx_default_leverage,
            req.okx_trade_mode.trim(),
            req.okx_position_mode.trim(),
        );

        std::fs::write(".env", content)?;
        info!("Successfully saved configuration to .env");

        // Reload new settings in-memory
        let config_path = settings_json_path();
        let new_settings = Settings::load_from_file_and_env(&config_path);

        let creds = if new_settings.is_okx_configured() {
            Some(OKXCredentials::new(
                &new_settings.okx.api_key,
                &new_settings.okx.secret_key,
                &new_settings.okx.passphrase,
            ))
        } else {
            None
        };

        let new_client = OKXClient::new(
            &new_settings.okx.base_url,
            creds,
            new_settings.okx.demo_trading,
            15,
        );

        let audit_path = Some(records_dir().join("trade_audit.jsonl"));
        let new_executor = OKXTradeExecutor::new(
            new_client.clone(),
            new_settings.okx.default_order_size,
            &new_settings.okx.trade_mode,
            &new_settings.okx.position_mode,
            new_settings.okx.default_leverage,
            new_settings.okx.block_new_entries_when_position_open,
            new_settings.general.decision_confidence_threshold,
            new_settings.okx.max_signal_age_seconds,
            new_settings.okx.max_pending_bars,
            audit_path,
        );

        let new_orchestrator = TwoStageOrchestrator::new(
            new_settings.clone(),
            records_dir(),
        );

        *self.current_trading_system.write() = new_settings.general.trading_system.clone();
        *self.settings.write() = new_settings;
        *self.okx_client.write() = new_client;
        *self.executor.write() = new_executor;
        *self.orchestrator.write() = new_orchestrator;

        Ok(self.get_config())
    }

    pub fn set_automation(
        &self,
        enabled: bool,
        symbol: &str,
        timeframe: &str,
        confirmation: &str,
        session_preset: Option<&str>,
        session_timezone: Option<&str>,
        session_start: Option<&str>,
        session_end: Option<&str>,
        session_weekdays: Option<&[u32]>,
        trading_system: Option<&str>,
    ) -> Result<Value> {
        let settings = self.settings.read();
        if enabled && !*self.automation_enabled.read() {
            let required = if settings.okx.demo_trading { "ENABLE DEMO" } else { "ENABLE LIVE" };
            if confirmation.trim().to_uppercase() != required {
                return Err(anyhow!("confirmation must be {}", required));
            }
            if !settings.is_okx_configured() {
                return Err(anyhow!("OKX API credentials are not configured"));
            }
        }

        let cur_session = self.automation_session.read().clone();
        let session = build_trading_session(
            session_preset.unwrap_or(&cur_session.preset),
            session_timezone.unwrap_or(&cur_session.timezone_name),
            session_start.unwrap_or(&format!("{:02}:{:02}", cur_session.start.hour(), cur_session.start.minute())),
            session_end.unwrap_or(&format!("{:02}:{:02}", cur_session.end.hour(), cur_session.end.minute())),
            session_weekdays.or(Some(&cur_session.weekdays)),
        );

        if let Some(sys) = trading_system {
            if !sys.trim().is_empty() {
                *self.current_trading_system.write() = sys.trim().to_string();
            }
        }

        *self.automation_enabled.write() = enabled;
        *self.automation_symbol.write() = symbol.trim().to_uppercase();
        *self.automation_timeframe.write() = timeframe.to_string();
        *self.automation_session.write() = session;

        drop(settings);
        Ok(self.status())
    }

    pub async fn fetch_raw_candles(&self, inst_id: &str, timeframe: &str, limit: usize) -> Result<Vec<KlineBar>> {
        let client = self.okx_client.read().clone();
        let raw_rows = client.get_candles(inst_id, timeframe, limit).await?;
        let mut bars = Vec::with_capacity(raw_rows.len());

        for (i, row) in raw_rows.iter().enumerate() {
            if row.len() < 6 { continue; }
            let ts = row[0].parse::<i64>().unwrap_or(0);
            let o = row[1].parse::<f64>().unwrap_or(0.0);
            let h = row[2].parse::<f64>().unwrap_or(0.0);
            let l = row[3].parse::<f64>().unwrap_or(0.0);
            let c = row[4].parse::<f64>().unwrap_or(0.0);
            let vol = row[5].parse::<f64>().unwrap_or(0.0);
            let closed = if row.len() > 8 { row[8] == "1" } else { i > 0 };

            bars.push(KlineBar {
                seq: i + 1,
                ts_open: ts,
                open: o,
                high: h,
                low: l,
                close: c,
                volume: vol,
                amount: 0.0,
                pct_chg: None,
                closed,
            });
        }
        Ok(bars)
    }

    pub async fn instruments(&self, inst_type: &str) -> Result<Vec<Value>> {
        let client = self.okx_client.read().clone();
        client.get_instruments(inst_type, None).await
    }

    pub async fn candles(&self, inst_id: &str, timeframe: &str, limit: usize) -> Result<Vec<KlineBar>> {
        let raw = self.fetch_raw_candles(inst_id, timeframe, limit.max(10).min(300)).await?;
        if let Some(frame) = build_live_frame(&raw, limit, inst_id, timeframe, None) {
            Ok(frame.bars)
        } else {
            Ok(raw)
        }
    }

    pub async fn account(&self) -> Result<Value> {
        let is_configured = self.settings.read().is_okx_configured();
        if !is_configured {
            return Ok(serde_json::json!({
                "configured": false,
                "summary": {},
                "equity_curve": [],
                "balances": [],
                "positions": [],
                "orders": [],
            }));
        }

        let client = self.okx_client.read().clone();
        let balance_rows = client.get_account_balance().await.unwrap_or_default();
        let position_rows = client.get_positions(None).await.unwrap_or_default();
        let mut pending_orders = client.get_pending_orders(None).await.unwrap_or_default();

        if let Ok(algos) = client.get_pending_algo_orders(None, "trigger").await {
            pending_orders.extend(algos);
        }

        let total_equity = balance_rows.first()
            .and_then(|b| b.get("totalEq"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        let upl = position_rows.iter()
            .filter_map(|p| p.get("upl").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()))
            .sum::<f64>();

        let summary = serde_json::json!({
            "total_equity_usd": total_equity,
            "available_equity_usd": total_equity,
            "unrealized_pnl": upl,
            "position_count": position_rows.len(),
            "pending_order_count": pending_orders.len(),
        });

        Ok(serde_json::json!({
            "configured": true,
            "summary": summary,
            "balances": balance_rows,
            "positions": position_rows,
            "orders": pending_orders,
        }))
    }

    pub async fn cancel_order(
        &self,
        inst_id: &str,
        ord_id: Option<&str>,
        cl_ord_id: Option<&str>,
        algo_id: Option<&str>,
    ) -> Result<Value> {
        let client = self.okx_client.read().clone();
        if let Some(aid) = algo_id {
            if !aid.is_empty() {
                return client.cancel_algo_order(inst_id, aid).await;
            }
        }
        client.cancel_order(inst_id, ord_id, cl_ord_id).await
    }

    pub async fn cancel_all_orders(&self, inst_id: Option<&str>) -> Result<usize> {
        let client = self.okx_client.read().clone();
        let regular_orders = client.get_pending_orders(inst_id).await.unwrap_or_default();
        let algo_orders = client.get_pending_algo_orders(inst_id, "trigger").await.unwrap_or_default();
        let mut cancelled_count = 0;

        for ord in regular_orders {
            let symbol = ord.get("instId").and_then(|v| v.as_str()).unwrap_or("");
            let ord_id = ord.get("ordId").and_then(|v| v.as_str());
            let cl_ord_id = ord.get("clOrdId").and_then(|v| v.as_str());
            if !symbol.is_empty() && (ord_id.is_some() || cl_ord_id.is_some()) {
                if client.cancel_order(symbol, ord_id, cl_ord_id).await.is_ok() {
                    cancelled_count += 1;
                }
            }
        }

        for algo in algo_orders {
            let symbol = algo.get("instId").and_then(|v| v.as_str()).unwrap_or("");
            let algo_id = algo.get("algoId").and_then(|v| v.as_str()).unwrap_or("");
            if !symbol.is_empty() && !algo_id.is_empty() {
                if client.cancel_algo_order(symbol, algo_id).await.is_ok() {
                    cancelled_count += 1;
                }
            }
        }

        Ok(cancelled_count)
    }

    pub fn decision_records(&self, limit: usize) -> Vec<Value> {
        let paths = list_record_paths(&records_dir());
        let mut records = Vec::new();
        for p in paths.into_iter().take(limit) {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if let Some(r) = load_record(&p) {
                let dec_obj = r.stage2_decision.as_ref();
                let inner_dec = dec_obj.and_then(|d| d.get("decision")).or(dec_obj);

                let direction = inner_dec.and_then(|d| d.get("order_direction")).and_then(|v| v.as_str()).unwrap_or("不下单").to_string();
                let order_type = inner_dec.and_then(|d| d.get("order_type")).and_then(|v| v.as_str()).unwrap_or("不下单").to_string();
                let confidence = inner_dec.and_then(|d| d.get("trade_confidence")).and_then(|v| v.as_u64());
                let entry_price = inner_dec.and_then(|d| d.get("entry_price")).and_then(|v| v.as_f64());
                let stop_loss_price = inner_dec.and_then(|d| d.get("stop_loss_price")).and_then(|v| v.as_f64());
                let take_profit_price = inner_dec.and_then(|d| d.get("take_profit_price")).and_then(|v| v.as_f64());
                let take_profit_price_2 = inner_dec.and_then(|d| d.get("take_profit_price_2")).and_then(|v| v.as_f64());
                let estimated_win_rate = inner_dec.and_then(|d| d.get("estimated_win_rate")).and_then(|v| v.as_f64());
                let reasoning = inner_dec.and_then(|d| d.get("reasoning")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let exception_str = r.exception.as_ref().map(|e| e.to_string());

                let item = serde_json::json!({
                    "id": stem,
                    "symbol": r.meta.symbol,
                    "timeframe": r.meta.timeframe,
                    "trading_system": r.meta.trading_system,
                    "timestamp_ms": r.meta.timestamp_local_ms,
                    "timestamp_iso": r.meta.timestamp_local_iso,
                    "direction": direction,
                    "order_type": order_type,
                    "confidence": confidence,
                    "entry_price": entry_price,
                    "stop_loss_price": stop_loss_price,
                    "take_profit_price": take_profit_price,
                    "take_profit_price_2": take_profit_price_2,
                    "estimated_win_rate": estimated_win_rate,
                    "reasoning": reasoning,
                    "exception": exception_str,
                    "meta": r.meta,
                    "stage1_diagnosis": r.stage1_diagnosis,
                    "stage2_decision": r.stage2_decision,
                    "usage": r.usage_total,
                });
                records.push(item);
            }
        }
        records
    }

    pub fn delete_decision_record(&self, record_id: &str) -> bool {
        delete_record(&records_dir(), record_id)
    }

    pub fn trade_records(&self, limit: usize) -> Vec<AuditEntry> {
        self.executor.read().audit_history(limit)
    }

    pub fn delete_trade_record(&self, record_id: &str) -> bool {
        self.executor.read().delete_audit_entry(record_id)
    }

    pub async fn analyze(
        &self,
        inst_id: &str,
        timeframe: &str,
        bar_count: usize,
        execute: bool,
        system_override: Option<&str>,
    ) -> Result<Value> {
        let system = match system_override {
            Some(s) if !s.trim().is_empty() => {
                let s_clean = s.trim().to_string();
                *self.current_trading_system.write() = s_clean.clone();
                s_clean
            }
            _ => self.current_trading_system.read().clone(),
        };

        let fetch_limit = (bar_count + INDICATOR_WARMUP_BARS + 20).min(300).max(100);
        let raw_bars = self.fetch_raw_candles(inst_id, timeframe, fetch_limit).await?;
        let frame = build_analysis_frame(&raw_bars, bar_count, inst_id, timeframe, None)
            .ok_or_else(|| anyhow!("not enough closed OKX candles to build {}-bar analysis", bar_count))?;

        let client = self.okx_client.read().clone();

        // 1. 实时获取 OKX 当前品种的活跃持仓状态与生效中的止盈止损
        let mut pos_ctx = PositionContext {
            has_position: false,
            symbol: inst_id.to_string(),
            pos_side: "none".to_string(),
            pos_size: "0".to_string(),
            mgn_mode: "cross".to_string(),
            ..Default::default()
        };

        if let Ok(positions) = client.get_positions(Some(inst_id)).await {
            for p in positions {
                let sz_str = p.get("pos").and_then(|v| v.as_str()).unwrap_or("0");
                if let Ok(sz) = sz_str.parse::<f64>() {
                    if sz.abs() > 1e-6 {
                        pos_ctx.has_position = true;
                        pos_ctx.pos_side = if sz > 0.0 { "long".to_string() } else { "short".to_string() };
                        pos_ctx.pos_size = sz.abs().to_string();
                        pos_ctx.open_avg_px = p.get("avgPx").and_then(|v| v.as_str()).and_then(|s| s.parse().ok());
                        pos_ctx.mark_px = p.get("markPx").and_then(|v| v.as_str()).and_then(|s| s.parse().ok());
                        pos_ctx.unrealized_pnl = p.get("upl").and_then(|v| v.as_str()).and_then(|s| s.parse().ok());
                        pos_ctx.unrealized_pnl_ratio = p.get("uplRatio").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).map(|r: f64| r * 100.0);
                        pos_ctx.leverage = p.get("lever").and_then(|v| v.as_str()).and_then(|s| s.parse().ok());
                        pos_ctx.mgn_mode = p.get("mgnMode").and_then(|v| v.as_str()).unwrap_or("cross").to_string();
                        pos_ctx.open_time_ms = p.get("cTime").and_then(|v| v.as_str()).and_then(|s| s.parse().ok());
                        break;
                    }
                }
            }
        }

        if pos_ctx.has_position {
            if let Ok(algos) = client.get_pending_algo_orders(Some(inst_id), "conditional").await {
                for a in algos {
                    if let Some(sl) = a.get("slTriggerPx").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()) {
                        pos_ctx.current_sl = Some(sl);
                    }
                    if let Some(tp) = a.get("tpTriggerPx").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()) {
                        pos_ctx.current_tp = Some(tp);
                    }
                    if let Some(aid) = a.get("algoId").and_then(|v| v.as_str()) {
                        pos_ctx.algo_id = Some(aid.to_string());
                    }
                }
            }
        }

        let record = {
            let orch = self.orchestrator.read().clone();
            orch.run_analysis_with_system_and_pos(&frame, &system, Some(&pos_ctx)).await?
        };

        let mut execution_res = Value::Null;
        if execute {
            if let Some(dec_wrap) = &record.stage2_decision {
                let dec = dec_wrap.get("decision").unwrap_or(dec_wrap);
                let order_type = dec.get("order_type").and_then(|v| v.as_str()).unwrap_or("");
                let action = dec.get("action").and_then(|v| v.as_str()).unwrap_or("");

                if ["限价单", "突破单", "市价单"].contains(&order_type) || action == "OPEN" {
                    let sig_ts = frame.bars.first().map(|b| b.ts_open).unwrap_or(0);
                    let executor = self.executor.read().clone();
                    let result = executor.execute(inst_id, timeframe, sig_ts, dec).await;
                    execution_res = serde_json::to_value(result).unwrap_or(Value::Null);
                } else if order_type == "平仓" || action == "CLOSE_EARLY" {
                    if pos_ctx.has_position {
                        info!("Executing CLOSE_EARLY for {}...", inst_id);
                        match client.close_position(inst_id, &pos_ctx.mgn_mode, None).await {
                            Ok(res) => {
                                execution_res = serde_json::json!({
                                    "submitted": true,
                                    "action": "CLOSE_EARLY",
                                    "symbol": inst_id,
                                    "reason": "AI 主动平仓 (CLOSE_EARLY) 离场成功",
                                    "response": res
                                });
                            }
                            Err(e) => {
                                execution_res = serde_json::json!({
                                    "submitted": false,
                                    "action": "CLOSE_EARLY",
                                    "symbol": inst_id,
                                    "reason": format!("AI 主动平仓失败: {}", e)
                                });
                            }
                        }
                    } else {
                        execution_res = serde_json::json!({
                            "submitted": false,
                            "action": "CLOSE_EARLY",
                            "symbol": inst_id,
                            "reason": "当前无持仓，无需执行平仓"
                        });
                    }
                } else if order_type == "修改止损" || action == "MOVE_STOP_LOSS" {
                    let new_sl = dec.get("new_stop_loss_price").and_then(|v| v.as_f64())
                        .or_else(|| dec.get("stop_loss_price").and_then(|v| v.as_f64()));

                    if let Some(n_sl) = new_sl {
                        if pos_ctx.has_position {
                            // 铁律校验：单向移损（多单只能上移，空单只能下移）
                            let is_long = pos_ctx.pos_side == "long";
                            let is_valid_trailing = match pos_ctx.current_sl {
                                Some(cur_sl) => {
                                    if is_long { n_sl > cur_sl } else { n_sl < cur_sl }
                                }
                                None => true,
                            };

                            if is_valid_trailing {
                                info!("Executing MOVE_STOP_LOSS for {} to {}...", inst_id, n_sl);
                                let mut amend_success = false;
                                if let Some(algo_id) = &pos_ctx.algo_id {
                                    if let Ok(res) = client.amend_algo_order(inst_id, algo_id, Some(n_sl), None).await {
                                        amend_success = true;
                                        execution_res = serde_json::json!({
                                            "submitted": true,
                                            "action": "MOVE_STOP_LOSS",
                                            "symbol": inst_id,
                                            "new_stop_loss": n_sl,
                                            "reason": format!("已成功修改 OKX 止损委托至 {}", n_sl),
                                            "response": res
                                        });
                                    }
                                }

                                if !amend_success {
                                    // 若无现有 algo 或修改失败，撤销同品种旧条件单并重新下达保护止损
                                    if let Ok(old_algos) = client.get_pending_algo_orders(Some(inst_id), "conditional").await {
                                        for a in old_algos {
                                            if let Some(aid) = a.get("algoId").and_then(|v| v.as_str()) {
                                                let _ = client.cancel_algo_order(inst_id, aid).await;
                                            }
                                        }
                                    }
                                    let close_side = if is_long { "sell" } else { "buy" };
                                    let algo_payload = serde_json::json!({
                                        "instId": inst_id,
                                        "tdMode": pos_ctx.mgn_mode,
                                        "side": close_side,
                                        "ordType": "conditional",
                                        "sz": pos_ctx.pos_size,
                                        "slTriggerPx": n_sl.to_string(),
                                        "slOrdPx": "-1"
                                    });
                                    match client.place_algo_order(&algo_payload).await {
                                        Ok(res) => {
                                            execution_res = serde_json::json!({
                                                "submitted": true,
                                                "action": "MOVE_STOP_LOSS",
                                                "symbol": inst_id,
                                                "new_stop_loss": n_sl,
                                                "reason": format!("已重新挂设保护止损至 {}", n_sl),
                                                "response": res
                                            });
                                        }
                                        Err(e) => {
                                            execution_res = serde_json::json!({
                                                "submitted": false,
                                                "action": "MOVE_STOP_LOSS",
                                                "symbol": inst_id,
                                                "reason": format!("设置保护止损失败: {}", e)
                                            });
                                        }
                                    }
                                }
                            } else {
                                execution_res = serde_json::json!({
                                    "submitted": false,
                                    "action": "MOVE_STOP_LOSS",
                                    "symbol": inst_id,
                                    "reason": format!("拒绝逆向扩大止损扛单！当前止损: {:?}, 目标止损: {}", pos_ctx.current_sl, n_sl)
                                });
                            }
                        } else {
                            execution_res = serde_json::json!({
                                "submitted": false,
                                "action": "MOVE_STOP_LOSS",
                                "symbol": inst_id,
                                "reason": "当前无持仓，无法移动止损"
                            });
                        }
                    }
                }
            }
        }

        let system_name = if system == "dog_walking" {
            "🐕 遛狗系统 (SMA 14/170 均线回归)"
        } else {
            "2PA 价格行为系统 (Al Brooks)"
        };

        let output = serde_json::json!({
            "symbol": inst_id,
            "timeframe": timeframe,
            "trading_system": system,
            "system_name": system_name,
            "signal_bar_ts": frame.bars.first().map(|b| b.ts_open).unwrap_or(0),
            "position_context": pos_ctx,
            "stage1": record.stage1_diagnosis,
            "stage2": record.stage2_decision,
            "decision": record.stage2_decision.as_ref().and_then(|d| d.get("decision")),
            "execution": execution_res,
            "usage": record.usage_total,
        });

        *self.latest_analysis.write() = Some(output.clone());
        Ok(output)
    }

    pub async fn automation_tick(&self) -> Result<()> {
        let auto_enabled = *self.automation_enabled.read();
        let session = self.automation_session.read().clone();
        if !auto_enabled || !session.is_open_at(Some(Utc::now())) {
            return Ok(());
        }

        let symbol = self.automation_symbol.read().clone();
        let timeframe = self.automation_timeframe.read().clone();

        let raw = match self.fetch_raw_candles(&symbol, &timeframe, 3).await {
            Ok(b) => b,
            Err(e) => {
                warn!("Automation tick failed to fetch candles: {}", e);
                return Ok(());
            }
        };

        if let Some(closed) = raw.iter().find(|b| b.closed) {
            let key = (symbol.clone(), timeframe.clone());
            let last_ts = self.last_closed_ts.read().get(&key).copied().unwrap_or(0);
            if last_ts == closed.ts_open {
                return Ok(());
            }

            self.last_closed_ts.write().insert(key, closed.ts_open);
            info!("New closed bar detected on {} ({}), triggering analysis...", symbol, timeframe);

            let bar_count = self.settings.read().general.analysis_bar_count;
            let system = self.current_trading_system.read().clone();
            if let Err(e) = self.analyze(&symbol, &timeframe, bar_count, true, Some(&system)).await {
                error!("Automation analysis error: {}", e);
            }
        }
        Ok(())
    }

    pub async fn get_contract_specs(&self, query_id: Option<&str>) -> Result<Value> {
        let client = self.okx_client.read().clone();
        let insts = client.get_instruments("SWAP", None).await.unwrap_or_default();
        let tickers = client.get_tickers("SWAP").await.unwrap_or_default();

        let mut ticker_map: HashMap<String, f64> = HashMap::new();
        for t in tickers {
            if let (Some(id), Some(last_str)) = (t.get("instId").and_then(|v| v.as_str()), t.get("last").and_then(|v| v.as_str())) {
                if let Ok(p) = last_str.parse::<f64>() {
                    ticker_map.insert(id.to_string(), p);
                }
            }
        }

        let popular_keys = [
            "BTC-USDT-SWAP", "ETH-USDT-SWAP", "SOL-USDT-SWAP", "DOGE-USDT-SWAP",
            "XRP-USDT-SWAP", "BNB-USDT-SWAP", "PEPE-USDT-SWAP", "SUI-USDT-SWAP",
            "XAU-USDT-SWAP", "XAG-USDT-SWAP", "AAPL-USDT-SWAP", "TSLA-USDT-SWAP",
            "NVDA-USDT-SWAP", "SPX-USDT-SWAP"
        ];

        let mut all_specs = Vec::new();
        let query_upper = query_id.map(|q| q.trim().to_uppercase()).unwrap_or_default();

        for inst in insts {
            let inst_id = inst.get("instId").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if inst_id.is_empty() { continue; }

            if !query_upper.is_empty() && !inst_id.contains(&query_upper) {
                continue;
            }

            let uly = inst.get("uly").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let ct_val: f64 = inst.get("ctVal").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(1.0);
            let ct_val_ccy = inst.get("ctValCcy").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let ct_type = inst.get("ctType").and_then(|v| v.as_str()).unwrap_or("linear").to_string();
            let min_sz: f64 = inst.get("minSz").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(1.0);
            let lot_sz: f64 = inst.get("lotSz").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(1.0);
            let tick_sz: f64 = inst.get("tickSz").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.1);
            let max_leverage: f64 = inst.get("lever").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(100.0);
            let state = inst.get("state").and_then(|v| v.as_str()).unwrap_or("live").to_string();

            let last_price = ticker_map.get(&inst_id).copied().unwrap_or(0.0);
            let usdt_per_contract = if ct_type == "inverse" {
                if last_price > 0.0 { ct_val / last_price } else { 0.0 }
            } else {
                ct_val * last_price
            };

            let is_popular = popular_keys.contains(&inst_id.as_str());

            all_specs.push(serde_json::json!({
                "inst_id": inst_id,
                "uly": uly,
                "ct_val": ct_val,
                "ct_val_ccy": ct_val_ccy,
                "ct_type": ct_type,
                "min_sz": min_sz,
                "lot_sz": lot_sz,
                "tick_sz": tick_sz,
                "max_leverage": max_leverage,
                "last_price": last_price,
                "usdt_per_contract": usdt_per_contract,
                "is_popular": is_popular,
                "state": state,
            }));
        }

        all_specs.sort_by(|a, b| {
            let a_pop = a.get("is_popular").and_then(|v| v.as_bool()).unwrap_or(false);
            let b_pop = b.get("is_popular").and_then(|v| v.as_bool()).unwrap_or(false);
            b_pop.cmp(&a_pop).then_with(|| {
                let a_id = a.get("inst_id").and_then(|v| v.as_str()).unwrap_or("");
                let b_id = b.get("inst_id").and_then(|v| v.as_str()).unwrap_or("");
                a_id.cmp(b_id)
            })
        });

        Ok(serde_json::json!({
            "total": all_specs.len(),
            "popular_count": popular_keys.len(),
            "specs": all_specs,
        }))
    }
}
