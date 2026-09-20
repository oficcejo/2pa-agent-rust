use crate::config::paths::{records_dir, settings_json_path};
use crate::config::settings::Settings;
use crate::data::base::{KlineBar, PositionContext};
use crate::data::snapshot::{build_analysis_frame, build_live_frame, INDICATOR_WARMUP_BARS};
use crate::learning::{
    compare_candidate, group_by_prompt_version, metrics_for, ExperienceWriter, OutcomeStore,
    PromptArtifactStore, Reconciler, STRATEGY_PROMPT_NAME,
};
use crate::okx::client::{OKXClient, OKXCredentials};
use crate::okx::trading::{AuditEntry, OKXTradeExecutor, BROKER_TAG};
use crate::orchestrator::two_stage::TwoStageOrchestrator;
use crate::records::history::{delete_record, list_record_paths, load_record};
use crate::util::mask::mask_secret;
use crate::web::sessions::{build_trading_session, TradingSession};
use anyhow::{anyhow, Context, Result};
use chrono::{Timelike, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Debug, Clone, Default, Serialize)]
pub struct AutomationRuntime {
    pub phase: String,
    pub message: String,
    pub last_tick_ms: Option<i64>,
    pub last_attempt_ms: Option<i64>,
    pub last_success_ms: Option<i64>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub next_retry_ms: Option<i64>,
}

/// Observability for the continual-learning loop.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LearningRuntime {
    pub last_reconcile_ms: Option<i64>,
    pub last_report: Option<Value>,
    pub last_error: Option<String>,
    pub reconcile_runs: u64,
}

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
    #[serde(default = "default_true")]
    pub okx_auto_order_sizing: bool,
    #[serde(default = "default_risk_percent")]
    pub okx_risk_percent: f64,
    #[serde(default = "default_max_margin_percent")]
    pub okx_max_margin_percent: f64,
    #[serde(default = "default_trade_mode")]
    pub okx_trade_mode: String,
    #[serde(default = "default_position_mode")]
    pub okx_position_mode: String,

    #[serde(default)]
    pub learning_enabled: Option<bool>,
    #[serde(default)]
    pub learning_daily_drawdown_limit_usd: Option<f64>,
    #[serde(default)]
    pub learning_shadow_trading_enabled: Option<bool>,
}

fn default_risk_percent() -> f64 { 2.0 }
fn default_max_margin_percent() -> f64 { 25.0 }

fn default_llm_base_url() -> String { "https://api.deepseek.com".to_string() }
fn default_llm_model() -> String { "deepseek-v4-flash".to_string() }
fn default_trading_system() -> String { "2pa".to_string() }
fn default_okx_base_url() -> String { "https://www.okx.com".to_string() }
fn default_true() -> bool { true }
fn default_order_size() -> f64 { 1.0 }
fn default_leverage() -> f64 { 3.0 }
fn default_trade_mode() -> String { "cross".to_string() }
fn default_position_mode() -> String { "net".to_string() }

/// Stamp a decision with its receipt linkage before execution.
///
/// Without this the audit row cannot be attributed to a decision or a prompt
/// revision, and the outcome reconciler has nothing to join on.
fn stamp_receipt(decision: &mut Value, record: &crate::records::schema::AnalysisRecord) {
    decision["decision_record_id"] = serde_json::json!(record.meta.record_id);
    decision["prompt_version"] = serde_json::json!(record.meta.prompt_version);
    decision["prompt_hash"] = serde_json::json!(record.meta.prompt_hash);
    if let Some(diag) = record.stage1_diagnosis.as_ref() {
        if let Some(cycle) = diag.get("cycle_position") {
            decision["cycle_position"] = cycle.clone();
        }
        if let Some(patterns) = diag.get("detected_patterns") {
            decision["detected_patterns"] = patterns.clone();
        }
    }
}

#[derive(Debug, Clone)]
pub struct BacktestJobRecord {
    pub status: crate::backtest::types::BacktestJobStatus,
    pub report: Option<crate::backtest::types::BacktestReport>,
}

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
    pub operation_lock: Arc<tokio::sync::Mutex<()>>,
    pub automation_runtime: Arc<RwLock<AutomationRuntime>>,
    automation_tick_lock: tokio::sync::Mutex<()>,
    pub outcome_store: OutcomeStore,
    pub experience_writer: ExperienceWriter,
    pub prompt_store: PromptArtifactStore,
    pub learning_runtime: Arc<RwLock<LearningRuntime>>,
    pub hook_pipeline: Arc<RwLock<crate::learning::HookPipeline>>,
    pub shadow_hook: Arc<crate::learning::ShadowTradingHook>,
    pub drawdown_guard: Arc<crate::learning::DailyDrawdownGuardHook>,
    pub backtest_jobs: Arc<RwLock<HashMap<String, BacktestJobRecord>>>,
    pub decision_cache: crate::backtest::cache::DecisionCache,
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
            settings.okx.auto_order_sizing,
            settings.okx.risk_percent,
            settings.okx.max_margin_percent,
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

        let initial_system = if settings.general.trading_system.trim().eq_ignore_ascii_case("alpha_pilot") {
            "2pa_trend".to_string()
        } else {
            settings.general.trading_system.clone()
        };

        let drawdown_limit = settings.learning.daily_drawdown_limit_usd;
        let drawdown_guard = Arc::new(crate::learning::DailyDrawdownGuardHook::new(drawdown_limit));
        if let Ok(tz) = settings.okx.automation_session_timezone.parse::<chrono_tz::Tz>() {
            use chrono::Offset;
            let offset_sec = chrono::Utc::now().with_timezone(&tz).offset().fix().local_minus_utc();
            drawdown_guard.set_timezone_offset_ms((offset_sec as i64) * 1000);
        }
        let shadow_hook = Arc::new(crate::learning::ShadowTradingHook::new());
        let mut pipeline = crate::learning::HookPipeline::new();
        pipeline.add_hook(drawdown_guard.clone());
        pipeline.add_hook(shadow_hook.clone());

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
            operation_lock: Arc::new(tokio::sync::Mutex::new(())),
            automation_runtime: Arc::new(RwLock::new(AutomationRuntime::default())),
            automation_tick_lock: tokio::sync::Mutex::new(()),
            outcome_store: OutcomeStore::new(crate::config::paths::outcomes_dir()),
            experience_writer: ExperienceWriter::new(crate::config::paths::experience_dir()),
            prompt_store: PromptArtifactStore::new(crate::config::paths::prompt_artifacts_dir()),
            learning_runtime: Arc::new(RwLock::new(LearningRuntime::default())),
            hook_pipeline: Arc::new(RwLock::new(pipeline)),
            shadow_hook,
            drawdown_guard,
            backtest_jobs: Arc::new(RwLock::new(HashMap::new())),
            decision_cache: crate::backtest::cache::DecisionCache::new(None),
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
            "can_execute": auto_enabled && settings.is_okx_configured() && (settings.okx.demo_trading || settings.okx.live_trading_acknowledged),
            "broker_tag": BROKER_TAG,
            "symbol": symbol,
            "timeframe": timeframe,
            "trading_system": trading_system,
            "available_trading_systems": [
                {
                    "id": "2pa_trend",
                    "name": "2PA 价格行为系统 (Al Brooks)",
                    "description": "已确认二次入场或突破回踩，须高周期同向"
                },
                {
                    "id": "dog_reversion",
                    "name": "🐕 遛狗系统 (SMA 14/170 均线回归)",
                    "description": "基于 14 狗绳与 170 主人均线偏离力学与均值回归"
                },
                {"id":"dog_trend", "name":"遛狗顺势回踩", "description":"SMA170 斜率与回踩确认，高周期同向"},
                {
                    "id": "adaptive",
                    "name": "🧠 自适应观察模式（不开新仓）",
                    "description": "仅观察，待三个独立策略积累成交验证后再评估切换"
                }
            ],
            "confidence_threshold": settings.general.decision_confidence_threshold,
            "default_order_size": settings.okx.default_order_size,
            "default_leverage": settings.okx.default_leverage,
            "auto_order_sizing": settings.okx.auto_order_sizing,
            "risk_percent": settings.okx.risk_percent,
            "max_margin_percent": settings.okx.max_margin_percent,
            "trade_mode": settings.okx.trade_mode,
            "position_mode": settings.okx.position_mode,
            "block_new_entries_when_position_open": settings.okx.block_new_entries_when_position_open,
            "max_pending_bars": settings.okx.max_pending_bars,
            "automation_session": session.as_dict(Some(Utc::now())),
            "automation_runtime": self.automation_runtime.read().clone(),
            "automation_session_presets": crate::web::sessions::session_preset_options(),
            "circuit_breaker": {
                "tripped": self.drawdown_guard.is_tripped(),
                "current_drawdown_usd": self.drawdown_guard.current_drawdown_usd(),
                "max_loss_usd": self.drawdown_guard.max_loss_usd(),
            },
            "shadow_trading": {
                "enabled": settings.learning.shadow_trading_enabled,
                "active_positions": self.shadow_hook.active_positions().len(),
                "closed_outcomes": self.shadow_hook.closed_outcomes().len(),
            },
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
            "okx_auto_order_sizing": settings.okx.auto_order_sizing,
            "okx_risk_percent": settings.okx.risk_percent,
            "okx_max_margin_percent": settings.okx.max_margin_percent,
            "okx_trade_mode": settings.okx.trade_mode,
            "okx_position_mode": settings.okx.position_mode,
        })
    }

    pub fn save_env_config(&self, req: &SaveEnvRequest) -> Result<Value> {
        let _guard = self.operation_lock.try_lock().map_err(|_| anyhow!("交易处理中，请稍后保存配置"))?;
        for field in [&req.llm_api_key, &req.llm_base_url, &req.llm_model, &req.trading_system,
            &req.okx_api_key, &req.okx_secret_key, &req.okx_passphrase, &req.okx_base_url,
            &req.okx_trade_mode, &req.okx_position_mode] {
            anyhow::ensure!(!field.contains(['\r','\n','\0']), "配置字段不得包含换行");
        }
        anyhow::ensure!(crate::strategies::canonical(&req.trading_system).is_some(), "未知交易系统");
        anyhow::ensure!(["net","long_short"].contains(&req.okx_position_mode.as_str()), "未知持仓模式");
        anyhow::ensure!(["cash","cross","isolated"].contains(&req.okx_trade_mode.as_str()), "未知保证金模式");
        anyhow::ensure!(req.okx_default_order_size.is_finite() && req.okx_default_order_size > 0.0
            && req.okx_default_leverage.is_finite() && req.okx_default_leverage >= 1.0
            && (0.1..=20.0).contains(&req.okx_risk_percent) && (1.0..=100.0).contains(&req.okx_max_margin_percent), "无效的仓位或风险参数");
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
OKX_AUTO_ORDER_SIZING={}
OKX_RISK_PERCENT={}
OKX_MAX_MARGIN_PERCENT={}
OKX_TRADE_MODE={}
OKX_POSITION_MODE={}
OKX_BLOCK_NEW_ENTRIES_WHEN_POSITION_OPEN=true
OKX_MAX_SIGNAL_AGE_SECONDS=120
OKX_MAX_PENDING_BARS=3

# ------------------------------ 交易时段 ------------------------------
OKX_AUTOMATION_SESSION_PRESET=always
OKX_AUTOMATION_SESSION_TIMEZONE=UTC

# ------------------------------ 持续学习与风控生命周期 ------------------------------
LEARNING_ENABLED={}
LEARNING_DAILY_DRAWDOWN_LIMIT_USD={}
LEARNING_SHADOW_TRADING_ENABLED={}
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
            self.settings.read().okx.live_trading_acknowledged,
            req.okx_default_order_size,
            req.okx_default_leverage,
            req.okx_auto_order_sizing,
            req.okx_risk_percent,
            req.okx_max_margin_percent,
            req.okx_trade_mode.trim(),
            req.okx_position_mode.trim(),
            req.learning_enabled.unwrap_or(self.settings.read().learning.enabled),
            req.learning_daily_drawdown_limit_usd.unwrap_or(self.settings.read().learning.daily_drawdown_limit_usd),
            req.learning_shadow_trading_enabled.unwrap_or(self.settings.read().learning.shadow_trading_enabled),
        );

        let token = self.settings.read().web_auth_token.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$");
        let content = format!("{}\nWEB_AUTH_TOKEN=\"{}\"\n", content, token);
        std::fs::write(".env", content)?;
        info!("Successfully saved configuration to .env");

        // Reload new settings in-memory
        let config_path = settings_json_path();
        let mut new_settings = Settings::load_from_file_and_env(&config_path);
        new_settings.web_auth_token = self.settings.read().web_auth_token.clone();
        // Apply this request directly; dotenv does not overwrite existing process variables.
        new_settings.provider.api_key = req.llm_api_key.trim().into();
        new_settings.provider.base_url = req.llm_base_url.trim().into();
        new_settings.provider.model = req.llm_model.trim().into();
        new_settings.provider.thinking = req.llm_thinking;
        new_settings.general.trading_system = req.trading_system.trim().into();
        new_settings.okx.api_key = req.okx_api_key.trim().into();
        new_settings.okx.secret_key = req.okx_secret_key.trim().into();
        new_settings.okx.passphrase = req.okx_passphrase.trim().into();
        new_settings.okx.base_url = req.okx_base_url.trim().into();
        new_settings.okx.demo_trading = req.okx_demo_trading;
        new_settings.okx.live_trading_acknowledged = self.settings.read().okx.live_trading_acknowledged;
        new_settings.okx.default_order_size = req.okx_default_order_size;
        new_settings.okx.default_leverage = req.okx_default_leverage;
        new_settings.okx.auto_order_sizing = req.okx_auto_order_sizing;
        new_settings.okx.risk_percent = req.okx_risk_percent;
        new_settings.okx.max_margin_percent = req.okx_max_margin_percent;
        new_settings.okx.trade_mode = req.okx_trade_mode.trim().into();
        new_settings.okx.position_mode = req.okx_position_mode.trim().into();
        new_settings.okx.auto_trading_enabled = false;
        if let Some(en) = req.learning_enabled { new_settings.learning.enabled = en; }
        if let Some(dd) = req.learning_daily_drawdown_limit_usd {
            new_settings.learning.daily_drawdown_limit_usd = dd.abs();
            self.drawdown_guard.set_max_loss_usd(dd.abs());
        }
        if let Some(st) = req.learning_shadow_trading_enabled {
            new_settings.learning.shadow_trading_enabled = st;
        }
        if let Ok(tz) = new_settings.okx.automation_session_timezone.parse::<chrono_tz::Tz>() {
            use chrono::Offset;
            let offset_sec = chrono::Utc::now().with_timezone(&tz).offset().fix().local_minus_utc();
            self.drawdown_guard.set_timezone_offset_ms((offset_sec as i64) * 1000);
        }
        *self.automation_enabled.write() = false;

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
            new_settings.okx.auto_order_sizing,
            new_settings.okx.risk_percent,
            new_settings.okx.max_margin_percent,
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
        if enabled {
            anyhow::ensure!(settings.okx.demo_trading || settings.okx.live_trading_acknowledged, "实盘执行未授权，请在服务器设置 OKX_LIVE_TRADING_ACKNOWLEDGED=true");
        }
        if enabled && !*self.automation_enabled.read() {
            let required = if settings.okx.demo_trading { "ENABLE DEMO" } else { "ENABLE LIVE" };
            if confirmation.trim().to_uppercase() != required {
                return Err(anyhow!("confirmation must be {}", required));
            }
            if !settings.is_okx_configured() {
                return Err(anyhow!("OKX API credentials are not configured"));
            }
        }

        if enabled && session_preset == Some("custom") {
            if let Some(tz) = session_timezone { anyhow::ensure!(tz.parse::<chrono_tz::Tz>().is_ok(), "无效时区"); }
            for time in [session_start, session_end].into_iter().flatten() {
                anyhow::ensure!(chrono::NaiveTime::parse_from_str(time, "%H:%M").is_ok(), "无效交易时段");
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
                *self.current_trading_system.write() = crate::strategies::canonical(sys).ok_or_else(|| anyhow!("未知交易系统"))?.to_string();
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
        let raw_rows = if limit > 300 {
            client.get_candles_paginated(inst_id, timeframe, limit, false).await?
        } else {
            client.get_candles(inst_id, timeframe, limit).await?
        };
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
        let raw = self.fetch_raw_candles(inst_id, timeframe, limit.clamp(10, 300)).await?;
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
        let balance_rows = client.get_account_balance().await?;
        let raw_positions = client.get_positions(None).await?;
        let mut position_rows = Vec::new();
        for p in raw_positions {
            let size = crate::web::positions::number(&p,"pos").ok_or_else(|| anyhow!("持仓数量无效"))?;
            if size != 0.0 { position_rows.push(crate::web::positions::normalize_position(&p)?); }
        }
        let mut pending_orders = client.get_pending_orders(None).await?;

        for kind in ["trigger", "conditional", "oco"] {
            pending_orders.extend(client.get_pending_algo_orders(None, kind).await?);
        }

        let total_equity = balance_rows.first()
            .and_then(|b| b.get("totalEq"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        let upl = position_rows.iter()
            .filter_map(|p| p.get("unrealized_pnl").and_then(Value::as_f64))
            .sum::<f64>();

        let available = balance_rows.first().and_then(|r| r["details"].as_array())
            .and_then(|rows| rows.iter().find(|r| r["ccy"] == "USDT"))
            .and_then(|r| crate::web::positions::number(r,"availEq").or_else(|| crate::web::positions::number(r,"availBal")));
        let updated_at = Utc::now().timestamp_millis();
        let equity_curve = {
            let mut history = self.equity_history.write();
            if history.last().and_then(|p| p["ts"].as_i64()).map(|t| updated_at - t >= 60_000).unwrap_or(true) {
                history.push(serde_json::json!({"ts": updated_at, "value":total_equity}));
                if history.len() > 1440 { history.remove(0); }
            }
            history.clone()
        };
        let balances: Vec<Value> = balance_rows.iter().filter_map(|r| r["details"].as_array()).flatten()
            .map(|r| serde_json::json!({"currency":r["ccy"], "equity":crate::web::positions::number(r,"eq"),
                "available":crate::web::positions::number(r,"availEq").or_else(|| crate::web::positions::number(r,"availBal")),
                "frozen":crate::web::positions::number(r,"frozenBal")})).collect();
        let summary = serde_json::json!({
            "updated_at_ms": updated_at,
            "total_equity_usd": total_equity,
            "available_equity_usd": available,
            "unrealized_pnl": upl,
            "position_count": position_rows.len(),
            "pending_order_count": pending_orders.len(),
        });

        Ok(serde_json::json!({
            "configured": true,
            "summary": summary,
            "balances": balances,
            "equity_curve": equity_curve,
            "equity_curve_scope": "current_process_observations",
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
        let _guard = self.operation_lock.lock().await;
        let client = self.okx_client.read().clone();
        if let Some(aid) = algo_id {
            if !aid.is_empty() {
                return client.cancel_algo_order(inst_id, aid).await;
            }
        }
        client.cancel_order(inst_id, ord_id, cl_ord_id).await
    }

    pub async fn cancel_all_orders(&self, inst_id: Option<&str>) -> Result<usize> {
        let _guard = self.operation_lock.lock().await;
        let client = self.okx_client.read().clone();
        let regular_orders = client.get_pending_orders(inst_id).await.unwrap_or_default();
        let algo_orders = client.get_pending_algo_orders(inst_id, "trigger").await.unwrap_or_default();
        let mut cancelled_count = 0;

        for ord in regular_orders {
            let symbol = ord.get("instId").and_then(|v| v.as_str()).unwrap_or("");
            let ord_id = ord.get("ordId").and_then(|v| v.as_str());
            let cl_ord_id = ord.get("clOrdId").and_then(|v| v.as_str());
            if !symbol.is_empty() && (ord_id.is_some() || cl_ord_id.is_some())
                && client.cancel_order(symbol, ord_id, cl_ord_id).await.is_ok()
            {
                cancelled_count += 1;
            }
        }

        for algo in algo_orders {
            let symbol = algo.get("instId").and_then(|v| v.as_str()).unwrap_or("");
            let algo_id = algo.get("algoId").and_then(|v| v.as_str()).unwrap_or("");
            if !symbol.is_empty() && !algo_id.is_empty()
                && client.cancel_algo_order(symbol, algo_id).await.is_ok()
            {
                cancelled_count += 1;
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
                let exception_str = r.exception.as_ref().and_then(|e| {
                    e.get("message").and_then(|v| v.as_str()).map(|s| s.to_string())
                        .or_else(|| e.as_str().map(|s| s.to_string()))
                        .or_else(|| Some(e.to_string()))
                });

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

    // ---- Continual-learning loop -------------------------------------------

    /// Resolve submitted orders into structured outcomes, then promote the
    /// qualified ones into the experience library.
    pub async fn reconcile_outcomes(&self) -> Result<Value> {
        let learning = self.settings.read().learning.clone();
        anyhow::ensure!(learning.enabled, "学习闭环未启用（将 LEARNING_ENABLED 设为 true 开启）");

        let entries = self.executor.read().audit_history(500);
        let reconciler = Reconciler::new(
            self.okx_client.read().clone(),
            self.outcome_store.clone(),
            self.experience_writer.clone(),
        );
        let report = reconciler
            .reconcile(
                &entries,
                &learning.qualification_policy(),
                learning.max_hold_bars,
                learning.lookback_hours.max(1) * 3_600_000,
                learning.write_experience,
            )
            .await;

        let value = serde_json::to_value(&report).unwrap_or(Value::Null);
        for outcome in &report.newly_resolved {
            let _ = self.hook_pipeline.read().run_post_outcome(&crate::learning::PostOutcomeContext { outcome: outcome.clone() });
        }
        let mut runtime = self.learning_runtime.write();
        runtime.last_reconcile_ms = Some(Utc::now().timestamp_millis());
        runtime.last_report = Some(value.clone());
        runtime.reconcile_runs = runtime.reconcile_runs.saturating_add(1);
        runtime.last_error = if report.errors.is_empty() { None } else { Some(report.errors.join("; ")) };
        Ok(value)
    }

    pub fn outcome_records(&self, limit: usize) -> Vec<crate::learning::TradeOutcome> {
        self.outcome_store.list(limit)
    }

    /// Aggregate view: loop state, realised metrics and per-version verdicts.
    pub fn learning_report(&self) -> Value {
        let learning = self.settings.read().learning.clone();
        let outcomes = self.outcome_store.list(1000);
        let policy = learning.evaluation_policy();
        let cur_sys = self.current_trading_system.read().clone();
        let active = crate::ai::prompt_assembler::resolve_strategy_prompt_for_system(&cur_sys, Some(
            &crate::config::paths::prompt_dir(),
        ));
        let grouped = group_by_prompt_version(&outcomes);
        let verdicts = crate::learning::evaluate_versions(&outcomes, &active.version, &policy);
        let multidim = crate::learning::compute_multidimensional_metrics(&outcomes);

        let versions: Vec<Value> = grouped
            .iter()
            .map(|(version, samples)| {
                serde_json::json!({
                    "version": version,
                    "metrics": metrics_for(samples),
                    "verdict": verdicts.get(version),
                })
            })
            .collect();

        serde_json::json!({
            "enabled": learning.enabled,
            "read_experience": learning.read_experience,
            "write_experience": learning.write_experience,
            "active_prompt": {
                "version": active.version,
                "hash": active.hash,
                "source": active.source,
            },
            "outcomes": {
                "total": outcomes.len(),
                "filled": outcomes.iter().filter(|o| o.filled).count(),
                "qualified": outcomes.iter().filter(|o| o.qualified).count(),
            },
            "overall": metrics_for(&outcomes),
            "by_strategy": crate::learning::group_by_strategy(&outcomes)
                .into_iter()
                .map(|(k, v)| (k, metrics_for(&v)))
                .collect::<std::collections::BTreeMap<_, _>>(),
            "by_symbol": crate::learning::group_by_symbol(&outcomes)
                .into_iter()
                .map(|(k, v)| (k, metrics_for(&v)))
                .collect::<std::collections::BTreeMap<_, _>>(),
            "multidimensional": multidim,
            "versions": versions,
            "experience": self.experience_writer.stats(),
            "policy": {
                "min_hold_bars": learning.min_hold_bars,
                "max_abs_r": learning.max_abs_r,
                "max_hold_bars": learning.max_hold_bars,
                "lookback_hours": learning.lookback_hours,
                "eval_min_samples": learning.eval_min_samples,
                "eval_min_expectancy_delta_r": learning.eval_min_expectancy_delta_r,
                "eval_max_win_rate_drop": learning.eval_max_win_rate_drop,
            },
            "runtime": *self.learning_runtime.read(),
        })
    }

    pub fn prompt_versions(&self) -> Value {
        let _ = self.prompt_store.ensure_seeded(
            STRATEGY_PROMPT_NAME,
            crate::ai::prompt_assembler::STRATEGY_PROMPT_FALLBACK,
        );
        let index = self.prompt_store.index(STRATEGY_PROMPT_NAME);
        serde_json::json!({
            "active": index.active,
            "versions": index.versions.values().collect::<Vec<_>>(),
        })
    }

    /// Publish a prompt revision as an inactive candidate.
    ///
    /// Publishing never changes live behaviour; activation is a separate step.
    pub fn publish_prompt_candidate(&self, content: &str, note: &str) -> Result<Value> {
        let trimmed = content.trim();
        anyhow::ensure!(!trimmed.is_empty(), "候选 prompt 不能为空");
        anyhow::ensure!(trimmed.len() >= 200, "候选 prompt 过短（至少 200 字符），疑似误提交");
        let version = self.prompt_store.publish(STRATEGY_PROMPT_NAME, content, note)?;
        Ok(serde_json::json!({
            "published": version,
            "active": self.prompt_store.index(STRATEGY_PROMPT_NAME).active,
            "note": "候选已发布但未激活，需通过选择性发布校验",
        }))
    }

    /// Activate a published revision, gated by the selective-publication policy.
    ///
    /// `force` bypasses the statistical gate for a revision that has no samples
    /// yet — a freshly published candidate cannot have any. The bypass is
    /// reported in the response rather than hidden.
    pub fn activate_prompt_version(&self, version: &str, force: bool) -> Result<Value> {
        let learning = self.settings.read().learning.clone();
        let mut verdict = Value::Null;

        if learning.enabled && !force {
            let outcomes = self.outcome_store.list(1000);
            let active_version = self.prompt_store.index(STRATEGY_PROMPT_NAME).active;
            let grouped = group_by_prompt_version(&outcomes);
            let baseline = metrics_for(grouped.get(&active_version).map(|v| v.as_slice()).unwrap_or(&[]));
            let candidate = metrics_for(grouped.get(version).map(|v| v.as_slice()).unwrap_or(&[]));
            let policy = learning.evaluation_policy();

            let (accepts, reasons) = if candidate.samples >= policy.min_samples && baseline.samples >= policy.min_samples {
                let result = compare_candidate(&candidate, &baseline, &policy);
                let mut v = serde_json::to_value(&result).unwrap_or(Value::Null);
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("evaluation_mode".into(), serde_json::json!("live_trades"));
                }
                verdict = v;
                (result.accepted, result.reasons)
            } else {
                // Break activation deadlock: fall back to ReplayEvaluator on benchmark episodes
                let benchmark_dir = crate::config::paths::benchmark_episodes_dir();
                let episodes = crate::records::benchmark::load_benchmark_episodes(&benchmark_dir)
                    .unwrap_or_default();
                if !episodes.is_empty() {
                    let cand_content = self.prompt_store.content(STRATEGY_PROMPT_NAME, version)
                        .ok_or_else(|| anyhow!("未找到候选版本 {}", version))?;
                    let base_content = self.prompt_store.content(STRATEGY_PROMPT_NAME, &active_version)
                        .unwrap_or_else(|| crate::ai::prompt_assembler::STRATEGY_PROMPT_FALLBACK.to_string());

                    let result = crate::learning::replay_evaluator::ReplayEvaluator::evaluate_candidate_vs_baseline(
                        &cand_content,
                        version,
                        &base_content,
                        &active_version,
                        &episodes,
                        &policy,
                    );
                    let mut v = serde_json::to_value(&result).unwrap_or(Value::Null);
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("evaluation_mode".into(), serde_json::json!("benchmark_replay"));
                        obj.insert("episodes_evaluated".into(), serde_json::json!(episodes.len()));
                    }
                    verdict = v;
                    (result.accepted, result.reasons)
                } else {
                    let result = compare_candidate(&candidate, &baseline, &policy);
                    verdict = serde_json::to_value(&result).unwrap_or(Value::Null);
                    (result.accepted, result.reasons)
                }
            };

            anyhow::ensure!(
                accepts,
                "候选版本未通过选择性发布：{}（确认无误可用 force=true 跳过统计校验）",
                reasons.join("；")
            );
        }

        let active = self.prompt_store.activate(STRATEGY_PROMPT_NAME, version)?;
        Ok(serde_json::json!({
            "active": active.version,
            "hash": active.hash,
            "forced": force,
            "verdict": verdict,
        }))
    }

    pub fn rollback_prompt_version(&self) -> Result<Value> {
        let active = self
            .prompt_store
            .rollback(STRATEGY_PROMPT_NAME)?
            .ok_or_else(|| anyhow!("已是最早版本，无法回退"))?;
        Ok(serde_json::json!({ "active": active.version, "hash": active.hash }))
    }

    /// Reset the daily drawdown circuit breaker manually.
    pub fn reset_circuit_breaker(&self) -> Value {
        self.drawdown_guard.reset();
        serde_json::json!({
            "reset": true,
            "tripped": false,
            "current_drawdown_usd": self.drawdown_guard.current_drawdown_usd(),
            "max_loss_usd": self.drawdown_guard.max_loss_usd(),
            "note": "单日风控熔断器已手动重置",
        })
    }

    /// Toggle shadow trading mode dynamically.
    pub fn toggle_shadow_trading(&self) -> Value {
        let mut settings = self.settings.write();
        let new_state = !settings.learning.shadow_trading_enabled;
        settings.learning.shadow_trading_enabled = new_state;
        serde_json::json!({
            "enabled": new_state,
            "shadow_trading_enabled": new_state,
            "note": if new_state { "影子交易模式已开启（新开仓订单将进入虚拟撮合通道）" } else { "影子交易模式已关闭（恢复真实/模拟盘执行）" },
        })
    }

    /// Propose a mutated prompt revision based on failure reflection (GEPA loop).
    pub fn propose_prompt_mutation(&self) -> Result<Value> {
        let outcomes = self.outcome_store.list(500);
        let failures: Vec<_> = outcomes.into_iter().filter(|o| o.r_multiple < 0.0 && o.qualified).collect();
        anyhow::ensure!(!failures.is_empty(), "当前尚无已对账的失败交易样本，无法进行失败归因与反思");

        let attributions = crate::learning::StrategyReflector::attribute_failures(&failures);
        let active_version = self.prompt_store.index(STRATEGY_PROMPT_NAME).active;
        let incumbent_content = self.prompt_store.content(STRATEGY_PROMPT_NAME, &active_version)
            .unwrap_or_else(|| crate::ai::prompt_assembler::STRATEGY_PROMPT_FALLBACK.to_string());

        let proposal = crate::learning::Proposer::propose_mutation(
            &incumbent_content,
            &active_version,
            &attributions,
        ).ok_or_else(|| anyhow!("无可用于生成反思突变的有效归因条款"))?;

        let published_version = self.prompt_store.publish(
            STRATEGY_PROMPT_NAME,
            &proposal.mutated_content,
            &proposal.diff_summary,
        )?;

        Ok(serde_json::json!({
            "proposal_id": proposal.proposal_id,
            "base_version": proposal.base_version,
            "published_version": published_version,
            "diff_summary": proposal.diff_summary,
            "addressed_modes": proposal.addressed_modes,
            "attributions_count": attributions.len(),
            "note": "反思突变候选版本已生成并发布为未激活 Artifact，可进行基准重放评估",
        }))
    }

    /// Solidify a settled trade outcome into a benchmark episode (data flywheel).
    pub fn solidify_outcome_to_episode(&self, signal_id: &str) -> Result<Value> {
        let outcome = self.outcome_store.load(signal_id)
            .ok_or_else(|| anyhow!("未找到交易结果: {}", signal_id))?;

        let benchmark_dir = crate::config::paths::benchmark_episodes_dir();
        let records = crate::records::history::list_record_paths(&records_dir());
        let mut klines: Vec<crate::data::base::KlineBar> = Vec::new();
        for path in records {
            if let Some(rec) = crate::records::history::load_record(&path) {
                if rec.meta.record_id == outcome.decision_record_id {
                    klines = rec
                        .kline_data
                        .iter()
                        .filter_map(|v| serde_json::from_value::<crate::data::base::KlineBar>(v.clone()).ok())
                        .collect();
                    break;
                }
            }
        }

        let saved_path = crate::learning::Trade2Episode::solidify(
            &benchmark_dir,
            &outcome,
            klines,
            vec![],
        )?;

        Ok(serde_json::json!({
            "success": true,
            "signal_id": signal_id,
            "saved_path": saved_path.to_string_lossy(),
            "note": "实盘样本已一键固化为离线基准切片",
        }))
    }

    /// List all offline benchmark episodes.
    pub fn list_benchmark_episodes(&self) -> Result<Value> {
        let dir = crate::config::paths::benchmark_episodes_dir();
        let episodes = crate::records::benchmark::load_benchmark_episodes(&dir)?;
        Ok(serde_json::to_value(episodes)?)
    }

    pub async fn analyze(
        &self,
        inst_id: &str,
        timeframe: &str,
        bar_count: usize,
        execute: bool,
        system_override: Option<&str>,
    ) -> Result<Value> {
        let _operation_guard = self.operation_lock.lock().await;
        if execute { self.ensure_execution_enabled()?; }
        anyhow::ensure!((20..=800).contains(&bar_count), "分析 K 线数量必须在 20 至 800 之间");
        let system = match system_override {
            Some(s) if !s.trim().is_empty() => {
                let s_clean = crate::strategies::canonical(s).ok_or_else(|| anyhow!("未知交易系统"))?.to_string();
                *self.current_trading_system.write() = s_clean.clone();
                s_clean
            }
            _ => self.current_trading_system.read().clone(),
        };

        let system = crate::strategies::canonical(&system).ok_or_else(|| anyhow!("未知交易系统"))?.to_string();
        if execute {
            let pre_anal_ctx = crate::learning::PreAnalysisContext {
                symbol: inst_id.to_string(),
                timeframe: timeframe.to_string(),
                trading_system: system.clone(),
                timestamp_ms: Utc::now().timestamp_millis(),
                account_equity_usd: self.equity_history.read().last().and_then(|e| e["value"].as_f64()),
            };
            self.hook_pipeline.read().run_pre_analysis(&pre_anal_ctx)?;
        }
        let is_adaptive = system.eq_ignore_ascii_case("adaptive") || system.contains("自适应");

        let fetch_limit = if is_adaptive {
            800.max(bar_count + INDICATOR_WARMUP_BARS + 20)
        } else {
            (bar_count + INDICATOR_WARMUP_BARS + 20).max(100)
        };
        let raw_bars = self.fetch_raw_candles(inst_id, timeframe, fetch_limit).await.context("读取分析行情失败")?;
        let frame_bars = bar_count;
        let frame = build_analysis_frame(&raw_bars, frame_bars, inst_id, timeframe, None)
            .ok_or_else(|| anyhow!("not enough closed OKX candles to build {}-bar analysis", frame_bars))?;

        let client = self.okx_client.read().clone();

        let position_mode = self.settings.read().okx.position_mode.clone();
        let configured = self.settings.read().is_okx_configured();
        let pos_ctx = if configured {
            crate::web::positions::read_position(&client, inst_id, &position_mode).await.context("读取账户持仓失败")?
        } else { PositionContext { symbol: inst_id.into(), pos_side: "none".into(), pos_size: "0".into(), ..Default::default() } };

        // 2. 获取高时间框架 (HTF) 宏观共振背景
        let htf_tf = if timeframe == "1m" || timeframe == "3m" || timeframe == "5m" || timeframe == "15m" {
            Some("1H")
        } else if timeframe == "30m" || timeframe == "1h" {
            Some("4H")
        } else {
            None
        };

        let mut htf_context_str = None;
        let mut structured_htf = None;
        if let Some(htf) = htf_tf {
            if let Ok(htf_bars) = self.fetch_raw_candles(inst_id, htf, 220).await {
                if let Some(htf_frame) = build_analysis_frame(&htf_bars, 20, inst_id, htf, None) {
                    structured_htf = Some(htf_frame.clone());
                    if let Some(latest) = htf_frame.bars.first() {
                        let htf_close = latest.close;
                        let htf_ema20 = htf_frame.indicators.ema20.first().copied().unwrap_or(0.0);
                        let htf_sma170 = htf_frame.indicators.sma170.first().copied().unwrap_or(0.0);
                        let htf_trend = if htf_ema20 > 0.0 {
                            if htf_close > htf_ema20 { "偏多 (Bullish, 位于 HTF EMA20 之上)" } else { "偏空 (Bearish, 位于 HTF EMA20 之下)" }
                        } else {
                            "中性震荡"
                        };
                        htf_context_str = Some(format!(
                            "- **HTF 周期**: {}\n\
                             - **最新收盘价**: {:.4}\n\
                             - **HTF EMA20**: {:.4}\n\
                             - **HTF SMA170**: {:.4}\n\
                             - **宏观格局偏向**: {}\n\
                             - **共振交易指引**: 顺大做小。低级别入场信号若与 HTF 趋势共振（如 15m 多单 + 1H 偏多），仍需结构和成本校验；回归策略须检查强逆向趋势，不能把共振当作实测胜率。",
                            htf, htf_close, htf_ema20, htf_sma170, htf_trend
                        ));
                    }
                }
            }
        }

        let record = {
            let orch = self.orchestrator.read().clone();
            orch.run_analysis_with_market_context(&frame, &system, Some(&pos_ctx), htf_context_str.as_deref(), structured_htf.as_ref()).await?
        };

        let mut execution_res = Value::Null;
        if execute {
            self.ensure_execution_enabled()?;
            if let Some(wrapper) = &record.stage2_decision {
                let dec = wrapper.get("decision").unwrap_or(wrapper);
                let order_type = dec["order_type"].as_str().unwrap_or("");
                let action = dec["action"].as_str().unwrap_or("");
                let executor = self.executor.read().clone();
                let timestamp = frame.bars.first().map(|b| b.ts_open).unwrap_or(0);
                if ["限价单", "突破单", "市价单"].contains(&order_type) && !["HOLD","WAIT","CLOSE_EARLY"].contains(&action) {
                    let mut decision = dec.clone();
                    if let Some(atr) = frame.indicators.atr14.first().filter(|a| a.is_finite() && **a > 0.0) {
                        decision["atr14"] = serde_json::json!(atr);
                    }
                    stamp_receipt(&mut decision, &record);
                    // Lifecycle hook check: e.g. circuit breaker, shadow intercept
                    let is_shadow = self.settings.read().learning.shadow_trading_enabled;
                    let current_balance = self.equity_history.read().last().and_then(|e| e["value"].as_f64()).unwrap_or(10000.0);
                    let pre_exec_ctx = crate::learning::PreExecutionContext {
                        symbol: inst_id.to_string(),
                        timeframe: timeframe.to_string(),
                        trading_system: system.clone(),
                        decision: decision.clone(),
                        account_balance_usd: current_balance,
                        is_shadow_mode: is_shadow,
                    };
                    let hook_decision = self.hook_pipeline.read().run_pre_execution(&pre_exec_ctx);
                    match hook_decision {
                        Ok(crate::learning::HookAction::Proceed) => {
                            let result = executor.execute(inst_id, timeframe, timestamp, &decision).await;
                            execution_res = serde_json::to_value(result)?;
                        }
                        Ok(crate::learning::HookAction::InterceptShadow { shadow_order_id, simulated_entry_price, note }) => {
                            execution_res = serde_json::json!({
                                "submitted": false,
                                "shadow": true,
                                "shadow_order_id": shadow_order_id,
                                "entry_price": simulated_entry_price,
                                "reason": note,
                            });
                        }
                        Ok(crate::learning::HookAction::Skip(reason)) => {
                            execution_res = serde_json::json!({
                                "submitted": false,
                                "reason": format!("Hook 跳过执行: {reason}"),
                            });
                        }
                        Err(rejection) => {
                            execution_res = serde_json::json!({
                                "submitted": false,
                                "reason": format!("{rejection}"),
                            });
                        }
                    }
                } else if ["平仓","修改止损","修改止盈","修改止盈止损"].contains(&order_type)
                    || ["CLOSE_EARLY","MOVE_STOP_LOSS","MOVE_TAKE_PROFIT","TRAILING_TAKE_PROFIT","MOVE_SL_TP"].contains(&action) {
                    let mut stamped = dec.clone();
                    stamp_receipt(&mut stamped, &record);
                    let is_shadow = self.settings.read().learning.shadow_trading_enabled;
                    if is_shadow {
                        execution_res = serde_json::json!({
                            "submitted": false,
                            "shadow": true,
                            "action": action,
                            "order_type": order_type,
                            "reason": "影子持仓管理指令已在虚拟通道执行",
                        });
                    } else {
                        let result = crate::web::positions::execute_management(&client, inst_id, &position_mode, &stamped).await;
                        let execution = crate::okx::trading::ExecutionResult {
                            submitted: result.is_ok(),
                            signal_id: OKXTradeExecutor::generate_signal_id(inst_id,timeframe,timestamp,&stamped),
                            request: serde_json::json!({"instId": inst_id, "action": action, "ordType": order_type}),
                            response: result.as_ref().ok().cloned(),
                            reason: result.err().map(|e| e.to_string()).unwrap_or_else(|| "持仓管理请求已提交".into()),
                            error_code: String::new(), broker_tag: BROKER_TAG.into(),
                        };
                        executor.audit(&execution, inst_id,timeframe,dec);
                        execution_res = serde_json::to_value(execution)?;
                    }
                }
            }
        }

        let system_name = if system == "dog_reversion" {
            "🐕 遛狗系统 (SMA 14/170 均线回归)"
        } else if system == "dog_trend" {
            "遛狗顺势回踩"
        } else if system == "adaptive" {
            "🧠 自适应观察模式（不开新仓）"
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

    pub fn ensure_execution_enabled(&self) -> Result<()> {
        let settings = self.settings.read();
        anyhow::ensure!(settings.is_okx_configured(), "OKX 凭据未配置");
        anyhow::ensure!(settings.okx.demo_trading || settings.okx.live_trading_acknowledged, "实盘执行未授权");
        anyhow::ensure!(*self.automation_enabled.read(), "交易执行开关未开启");
        Ok(())
    }

    pub async fn automation_tick(&self) -> Result<()> {
        let Ok(_tick_guard) = self.automation_tick_lock.try_lock() else { return Ok(()); };
        let now = Utc::now().timestamp_millis();
        self.automation_runtime.write().last_tick_ms = Some(now);
        if !*self.automation_enabled.read() {
            self.set_automation_phase("disabled", "自动交易未启用");
            return Ok(());
        }
        if self.automation_runtime.read().next_retry_ms.is_some_and(|t| now < t) {
            return Ok(());
        }
        let result = self.run_automation_tick().await;
        if let Err(error) = &result {
            let mut message = format!("{error:#}");
            {
                let settings = self.settings.read();
                for secret in [&settings.provider.api_key, &settings.okx.api_key,
                    &settings.okx.secret_key, &settings.okx.passphrase, &settings.web_auth_token] {
                    if !secret.is_empty() { message = message.replace(secret, "[redacted]"); }
                }
            }
            message = message.chars().take(2000).collect();
            {
                let mut runtime = self.automation_runtime.write();
                runtime.consecutive_failures = runtime.consecutive_failures.saturating_add(1);
                let delay = (30_000_i64 * (1_i64 << runtime.consecutive_failures.min(4).saturating_sub(1))).min(300_000);
                runtime.phase = "error".into();
                runtime.message = "自动运行失败，稍后重试；本次未完成决策".into();
                runtime.last_error = Some(message.clone());
                runtime.next_retry_ms = Some(Utc::now().timestamp_millis() + delay);
            }
            let record = crate::records::schema::AnalysisRecord {
                meta: crate::records::schema::RecordMeta {
                    timestamp_local_iso: crate::util::timefmt::now_local_iso(),
                    timestamp_local_ms: crate::util::timefmt::now_local_ms(),
                    symbol: self.automation_symbol.read().clone(),
                    timeframe: self.automation_timeframe.read().clone(),
                    bar_count: 0,
                    ai_provider: Value::Null,
                    decision_stance: self.settings.read().general.decision_stance.clone(),
                    trading_system: self.current_trading_system.read().clone(),
                    record_id: uuid::Uuid::new_v4().simple().to_string(),
                    prompt_version: String::new(),
                    prompt_hash: String::new(),
                },
                kline_data: vec![], htf_text: String::new(),
                stage1_messages: vec![], stage1_response: None, stage1_diagnosis: None,
                stage2_messages: vec![], stage2_response: None, stage2_decision: None,
                strategy_files_used: vec![], experience_loaded: vec![], position_context: None,
                exception: Some(serde_json::json!({"stage":"automation", "message":message})),
                usage_total: Value::Null,
            };
            let dir = self.orchestrator.read().records_dir.clone();
            if let Err(save_error) = crate::records::history::save_record(&dir, &record) {
                warn!("Failed to save automation failure record: {}", save_error);
                self.automation_runtime.write().last_error = Some(format!("{}；错误记录也无法保存：{}", message, save_error));
            }
        }
        result
    }

    fn set_automation_phase(&self, phase: &str, message: &str) {
        let mut runtime = self.automation_runtime.write();
        runtime.phase = phase.into();
        runtime.message = message.into();
    }

    async fn run_automation_tick(&self) -> Result<()> {
        let auto_enabled = *self.automation_enabled.read();
        let session = self.automation_session.read().clone();
        if !auto_enabled { return Ok(()); }
        self.ensure_execution_enabled()?;

        let symbol = self.automation_symbol.read().clone();
        let timeframe = self.automation_timeframe.read().clone();

        {
            self.set_automation_phase("checking_orders", "检查并清理到期入场挂单");
            let _guard = self.operation_lock.lock().await;
            let executor = self.executor.read().clone();
            executor.cancel_expired_entries(&symbol, &timeframe).await.context("检查或清理到期挂单失败")?;
        }
        if !session.is_open_at(Some(Utc::now())) {
            self.set_automation_phase("outside_session", "当前不在交易时段，等待时段开放");
            return Ok(());
        }

        // Run pre-analysis lifecycle hooks (e.g. daily drawdown circuit breaker)
        let pre_anal_ctx = crate::learning::PreAnalysisContext {
            symbol: symbol.clone(),
            timeframe: timeframe.clone(),
            trading_system: self.current_trading_system.read().clone(),
            timestamp_ms: Utc::now().timestamp_millis(),
            account_equity_usd: self.equity_history.read().last().and_then(|e| e["value"].as_f64()),
        };
        if let Err(rejection) = self.hook_pipeline.read().run_pre_analysis(&pre_anal_ctx) {
            let msg = format!("{rejection}");
            self.set_automation_phase("error", &msg);
            self.automation_runtime.write().last_error = Some(msg);
            return Ok(());
        }

        self.set_automation_phase("checking_candles", "检查最新收盘 K 线");
        let raw = self.fetch_raw_candles(&symbol, &timeframe, 3).await.context("读取最新行情失败")?;

        if let Some(closed) = raw.iter().find(|b| b.closed) {
            // Settle open shadow positions against the latest closed candle
            let resolved_shadows = self.shadow_hook.match_bar(&symbol, closed);
            for outcome in resolved_shadows {
                let _ = self.outcome_store.save(&outcome);
                let _ = self.hook_pipeline.read().run_post_outcome(&crate::learning::PostOutcomeContext { outcome: outcome.clone() });
                if self.settings.read().learning.write_experience && outcome.qualified {
                    let _ = self.experience_writer.record(&outcome);
                }
            }

            let key = (symbol.clone(), timeframe.clone());
            let last_ts = self.last_closed_ts.read().get(&key).copied().unwrap_or(0);
            if last_ts == closed.ts_open {
                self.set_automation_phase("waiting_bar", "本根 K 线已完成分析，等待下一根收盘");
                return Ok(());
            }

            info!("New closed bar detected on {} ({}), triggering analysis...", symbol, timeframe);

            let bar_count = self.settings.read().general.analysis_bar_count;
            let system = self.current_trading_system.read().clone();
            self.set_automation_phase("analyzing", "正在分析行情并生成决策");
            self.automation_runtime.write().last_attempt_ms = Some(Utc::now().timestamp_millis());
            let output = self.analyze(&symbol, &timeframe, bar_count, true, Some(&system)).await?;
            // Failed analysis must not consume this bar. Execution results (including
            // uncertain submissions) are returned normally and must never be retried.
            self.last_closed_ts.write().insert(key, closed.ts_open);
            let mut runtime = self.automation_runtime.write();
            runtime.phase = "waiting_bar".into();
            runtime.message = if output["execution"].is_null() {
                "决策已保存，本次观望或持有，没有提交交易；等待下一根收盘".into()
            } else {
                format!("决策已保存；执行结果：{}", output["execution"]["reason"].as_str().unwrap_or("请查看交易记录"))
            };
            runtime.last_success_ms = Some(Utc::now().timestamp_millis());
            runtime.last_error = None;
            runtime.consecutive_failures = 0;
            runtime.next_retry_ms = None;
        } else {
            anyhow::bail!("行情接口没有返回已收盘 K 线");
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

    pub fn submit_backtest_job(&self, config: crate::backtest::types::BacktestConfig) -> Result<String> {
        let job_id = format!("BT-{}", uuid::Uuid::new_v4().simple());
        let initial_status = crate::backtest::types::BacktestJobStatus {
            job_id: job_id.clone(),
            status: "running".to_string(),
            progress_pct: 0.0,
            current_bar: 0,
            total_bars: 0,
            message: "回测任务已创建，正在准备历史行情数据...".to_string(),
            error: None,
            created_at_ms: chrono::Utc::now().timestamp_millis(),
            completed_at_ms: None,
        };

        let record = BacktestJobRecord {
            status: initial_status.clone(),
            report: None,
        };

        // Maintain at most 30 completed/failed backtest records in memory
        {
            let mut jobs = self.backtest_jobs.write();
            if jobs.len() >= 30 {
                let mut removable: Vec<(String, i64)> = jobs
                    .iter()
                    .filter(|(_, r)| r.status.status == "completed" || r.status.status == "failed")
                    .map(|(k, r)| (k.clone(), r.status.created_at_ms))
                    .collect();
                removable.sort_by_key(|(_, ts)| *ts);
                for (old_k, _) in removable.into_iter().take(jobs.len() - 29) {
                    jobs.remove(&old_k);
                }
            }
            jobs.insert(job_id.clone(), record);
        }

        let jobs_map = Arc::clone(&self.backtest_jobs);
        let jid = job_id.clone();
        let cache = self.decision_cache.clone();
        let okx_client = self.okx_client.read().clone();
        let ai_client = self.orchestrator.read().ai_client.clone();

        tokio::spawn(async move {
            let jobs_map_cb = Arc::clone(&jobs_map);
            let jid_cb = jid.clone();
            let progress_cb = Arc::new(move |status: crate::backtest::types::BacktestJobStatus| {
                if let Some(r) = jobs_map_cb.write().get_mut(&jid_cb) {
                    r.status = status;
                }
            });

            // 1. Fetch or load historical bars
            let interval_ms = crate::backtest::timeframe_to_ms(&config.timeframe);
            let warmup_bars = 200usize;
            let start_ts = config.start_time_ms.unwrap_or(1710000000000);
            let candle_start_ts = start_ts - (warmup_bars as i64) * interval_ms;
            let trading_bars = match (config.start_time_ms, config.end_time_ms) {
                (Some(st), Some(et)) if et > st => ((et - st) / interval_ms).max(1) as usize,
                _ => config.max_bars.max(300),
            };
            let synth_count = (warmup_bars + trading_bars + 30).max(config.max_bars.max(500));

            let bars_res = if let Some(ref path_str) = config.fixture_path {
                crate::backtest::load_candles_from_file(std::path::Path::new(path_str))
            } else if config.data_source == crate::backtest::BacktestDataSource::LocalFile {
                Err(anyhow!("未指定本地行情文件路径 (fixture_path)"))
            } else if config.data_source == crate::backtest::BacktestDataSource::Synthetic {
                Ok(crate::backtest::generate_synthetic_candles_for_strategy(
                    &config.strategy_id,
                    synth_count,
                    65000.0,
                    interval_ms,
                    candle_start_ts,
                ))
            } else {
                let requested = synth_count.min(3000);
                let okx_res = crate::backtest::fetch_candles_okx(
                    &okx_client,
                    &config.symbol,
                    &config.timeframe,
                    requested,
                    config.end_time_ms,
                ).await;

                match okx_res {
                    Ok(b) if b.len() >= 200 => Ok(b),
                    Ok(b) => {
                        tracing::warn!("从 OKX 获取到 {} 根 K 线（不足 200 根预热要求），自动无缝切换为高质量仿真行情", b.len());
                        Ok(crate::backtest::generate_synthetic_candles_for_strategy(
                            &config.strategy_id,
                            synth_count,
                            65000.0,
                            interval_ms,
                            candle_start_ts,
                        ))
                    }
                    Err(err) => {
                        tracing::warn!("OKX 历史行情获取失败 ({})，自动切换为高质量仿真行情以保证回测顺利完成", err);
                        Ok(crate::backtest::generate_synthetic_candles_for_strategy(
                            &config.strategy_id,
                            synth_count,
                            65000.0,
                            interval_ms,
                            candle_start_ts,
                        ))
                    }
                }
            };

            let bars_asc = match bars_res {
                Ok(b) => b,
                Err(err) => {
                    let mut st = initial_status.clone();
                    st.status = "failed".to_string();
                    st.message = format!("获取历史 K 线失败: {}", err);
                    st.error = Some(err.to_string());
                    st.completed_at_ms = Some(chrono::Utc::now().timestamp_millis());
                    if let Some(r) = jobs_map.write().get_mut(&jid) {
                        r.status = st;
                    }
                    return;
                }
            };

            // 2. Run engine
            let engine = crate::backtest::BacktestEngine::new(config.clone(), cache, Some(Arc::new(ai_client)));
            match engine.run(&jid, &bars_asc, Some(progress_cb)).await {
                Ok(report) => {
                    if let Some(r) = jobs_map.write().get_mut(&jid) {
                        r.status.status = "completed".to_string();
                        r.status.progress_pct = 100.0;
                        r.status.message = "回测完成".to_string();
                        r.status.completed_at_ms = Some(chrono::Utc::now().timestamp_millis());
                        r.report = Some(report);
                    }
                }
                Err(err) => {
                    let mut st = initial_status.clone();
                    st.status = "failed".to_string();
                    st.message = format!("回测运行失败: {}", err);
                    st.error = Some(err.to_string());
                    st.completed_at_ms = Some(chrono::Utc::now().timestamp_millis());
                    if let Some(r) = jobs_map.write().get_mut(&jid) {
                        r.status = st;
                    }
                }
            }
        });

        Ok(job_id)
    }

    pub fn get_backtest_status(&self, job_id: &str) -> Option<crate::backtest::types::BacktestJobStatus> {
        self.backtest_jobs.read().get(job_id).map(|r| r.status.clone())
    }

    pub fn get_backtest_report(&self, job_id: &str) -> Option<crate::backtest::types::BacktestReport> {
        self.backtest_jobs.read().get(job_id).and_then(|r| r.report.clone())
    }
}
