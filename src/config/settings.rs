use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIProviderSettings {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_false")]
    pub thinking: bool,
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,
    #[serde(default = "default_context_window")]
    pub context_window: usize,
    #[serde(default = "default_stage_timeout_seconds")]
    pub stage_timeout_seconds: u64,
}

fn default_model() -> String { "deepseek-v4-flash".to_string() }
fn default_base_url() -> String { "https://api.deepseek.com".to_string() }
fn default_false() -> bool { false }
fn default_true() -> bool { true }
fn default_reasoning_effort() -> String { "high".to_string() }
fn default_context_window() -> usize { 128_000 }
fn default_stage_timeout_seconds() -> u64 { 240 }

impl Default for AIProviderSettings {
    fn default() -> Self {
        Self {
            model: default_model(),
            base_url: default_base_url(),
            api_key: String::new(),
            thinking: false,
            reasoning_effort: default_reasoning_effort(),
            context_window: default_context_window(),
            stage_timeout_seconds: default_stage_timeout_seconds(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralSettings {
    #[serde(default = "default_analysis_bar_count")]
    pub analysis_bar_count: usize,
    #[serde(default = "default_confidence_threshold")]
    pub decision_confidence_threshold: u32,
    #[serde(default = "default_decision_stance")]
    pub decision_stance: String,
    #[serde(default = "default_trading_system")]
    pub trading_system: String,
    #[serde(default)]
    pub enable_next_bar_prediction: bool,
    #[serde(default = "default_cooldown_bars")]
    pub structure_flip_cooldown_bars: usize,
}

fn default_analysis_bar_count() -> usize { 100 }
fn default_confidence_threshold() -> u32 { 40 }
fn default_decision_stance() -> String { "balanced".to_string() }
fn default_trading_system() -> String { "2pa".to_string() }
fn default_cooldown_bars() -> usize { 3 }

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            analysis_bar_count: default_analysis_bar_count(),
            decision_confidence_threshold: default_confidence_threshold(),
            decision_stance: default_decision_stance(),
            trading_system: default_trading_system(),
            enable_next_bar_prediction: false,
            structure_flip_cooldown_bars: default_cooldown_bars(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSettings {
    #[serde(default)]
    pub stage2_load_full_strategy_library: bool,
    #[serde(default)]
    pub experience_max_entries: usize,
    #[serde(default = "default_experience_max_chars")]
    pub experience_max_chars_per_entry: usize,
    #[serde(default = "default_true")]
    pub stage1_inject_pattern_briefs: bool,
}

fn default_experience_max_chars() -> usize { 400 }

impl Default for PromptSettings {
    fn default() -> Self {
        Self {
            stage2_load_full_strategy_library: false,
            experience_max_entries: 0,
            experience_max_chars_per_entry: default_experience_max_chars(),
            stage1_inject_pattern_briefs: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationSettings {
    #[serde(default = "default_normalization_mode")]
    pub normalization_mode: String,
    #[serde(default)]
    pub stage1_coherence_checks: bool,
    #[serde(default)]
    pub stage2_coherence_checks: bool,
    #[serde(default)]
    pub trace_semantic_checks: bool,
    #[serde(default)]
    pub strict_bar_by_bar_features: bool,
    #[serde(default)]
    pub disable_truncation_repair: bool,
    #[serde(default = "default_true")]
    pub retry_enabled: bool,
    #[serde(default = "default_retry_max")]
    pub retry_max: usize,
    #[serde(default = "default_retry_max_semantic")]
    pub retry_max_semantic: usize,
    #[serde(default = "default_true")]
    pub retry_stage2: bool,
}

fn default_normalization_mode() -> String { "lenient".to_string() }
fn default_retry_max() -> usize { 3 }
fn default_retry_max_semantic() -> usize { 1 }

impl Default for ValidationSettings {
    fn default() -> Self {
        Self {
            normalization_mode: default_normalization_mode(),
            stage1_coherence_checks: false,
            stage2_coherence_checks: false,
            trace_semantic_checks: false,
            strict_bar_by_bar_features: false,
            disable_truncation_repair: false,
            retry_enabled: true,
            retry_max: default_retry_max(),
            retry_max_semantic: default_retry_max_semantic(),
            retry_stage2: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OKXSettings {
    #[serde(default = "default_okx_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub secret_key: String,
    #[serde(default)]
    pub passphrase: String,
    #[serde(default = "default_true")]
    pub demo_trading: bool,
    #[serde(default)]
    pub auto_trading_enabled: bool,
    #[serde(default)]
    pub live_trading_acknowledged: bool,
    #[serde(default = "default_order_size")]
    pub default_order_size: f64,
    #[serde(default = "default_leverage")]
    pub default_leverage: f64,
    #[serde(default = "default_trade_mode")]
    pub trade_mode: String,
    #[serde(default = "default_position_mode")]
    pub position_mode: String,
    #[serde(default = "default_true")]
    pub block_new_entries_when_position_open: bool,
    #[serde(default = "default_max_signal_age")]
    pub max_signal_age_seconds: u64,
    #[serde(default = "default_max_pending_bars")]
    pub max_pending_bars: usize,
    #[serde(default = "default_automation_poll")]
    pub automation_poll_seconds: u64,
    #[serde(default = "default_session_preset")]
    pub automation_session_preset: String,
    #[serde(default = "default_session_timezone")]
    pub automation_session_timezone: String,
    #[serde(default = "default_session_start")]
    pub automation_session_start: String,
    #[serde(default = "default_session_end")]
    pub automation_session_end: String,
    #[serde(default = "default_session_weekdays")]
    pub automation_session_weekdays: Vec<u32>,
}

fn default_okx_base_url() -> String { "https://www.okx.com".to_string() }
fn default_order_size() -> f64 { 1.0 }
fn default_leverage() -> f64 { 3.0 }
fn default_trade_mode() -> String { "cross".to_string() }
fn default_position_mode() -> String { "net".to_string() }
fn default_max_signal_age() -> u64 { 120 }
fn default_max_pending_bars() -> usize { 3 }
fn default_automation_poll() -> u64 { 20 }
fn default_session_preset() -> String { "always".to_string() }
fn default_session_timezone() -> String { "UTC".to_string() }
fn default_session_start() -> String { "00:00".to_string() }
fn default_session_end() -> String { "00:00".to_string() }
fn default_session_weekdays() -> Vec<u32> { vec![0, 1, 2, 3, 4, 5, 6] }

impl Default for OKXSettings {
    fn default() -> Self {
        Self {
            base_url: default_okx_base_url(),
            api_key: String::new(),
            secret_key: String::new(),
            passphrase: String::new(),
            demo_trading: true,
            auto_trading_enabled: false,
            live_trading_acknowledged: false,
            default_order_size: default_order_size(),
            default_leverage: default_leverage(),
            trade_mode: default_trade_mode(),
            position_mode: default_position_mode(),
            block_new_entries_when_position_open: true,
            max_signal_age_seconds: default_max_signal_age(),
            max_pending_bars: default_max_pending_bars(),
            automation_poll_seconds: default_automation_poll(),
            automation_session_preset: default_session_preset(),
            automation_session_timezone: default_session_timezone(),
            automation_session_start: default_session_start(),
            automation_session_end: default_session_end(),
            automation_session_weekdays: default_session_weekdays(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub provider: AIProviderSettings,
    #[serde(default)]
    pub general: GeneralSettings,
    #[serde(default)]
    pub prompt: PromptSettings,
    #[serde(default)]
    pub validation: ValidationSettings,
    #[serde(default)]
    pub okx: OKXSettings,
}

impl Settings {
    pub fn load_from_file_and_env<P: AsRef<Path>>(path: P) -> Self {
        let _ = dotenvy::dotenv();

        let mut settings = if path.as_ref().exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => serde_json::from_str::<Settings>(&content).unwrap_or_default(),
                Err(_) => Settings::default(),
            }
        } else {
            Settings::default()
        };

        // Apply environment variable overrides
        if let Ok(v) = std::env::var("LLM_API_KEY").or_else(|_| std::env::var("AI_API_KEY")) {
            if !v.trim().is_empty() { settings.provider.api_key = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("LLM_BASE_URL").or_else(|_| std::env::var("AI_BASE_URL")) {
            if !v.trim().is_empty() { settings.provider.base_url = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("LLM_MODEL").or_else(|_| std::env::var("AI_MODEL")) {
            if !v.trim().is_empty() { settings.provider.model = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("LLM_THINKING").or_else(|_| std::env::var("AI_THINKING")) {
            settings.provider.thinking = v.trim().eq_ignore_ascii_case("true") || v.trim() == "1";
        }
        if let Ok(v) = std::env::var("LLM_REASONING_EFFORT").or_else(|_| std::env::var("AI_REASONING_EFFORT")) {
            if !v.trim().is_empty() { settings.provider.reasoning_effort = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("LLM_CONTEXT_WINDOW").or_else(|_| std::env::var("AI_CONTEXT_WINDOW")) {
            if let Ok(num) = v.trim().parse::<usize>() { settings.provider.context_window = num; }
        }
        if let Ok(v) = std::env::var("LLM_STAGE_TIMEOUT_SECONDS").or_else(|_| std::env::var("AI_STAGE_TIMEOUT_SECONDS")) {
            if let Ok(num) = v.trim().parse::<u64>() { settings.provider.stage_timeout_seconds = num; }
        }
        if let Ok(v) = std::env::var("TRADING_SYSTEM").or_else(|_| std::env::var("AI_TRADING_SYSTEM")) {
            if !v.trim().is_empty() { settings.general.trading_system = v.trim().to_string(); }
        }

        if let Ok(v) = std::env::var("OKX_API_KEY") {
            if !v.trim().is_empty() { settings.okx.api_key = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("OKX_SECRET_KEY") {
            if !v.trim().is_empty() { settings.okx.secret_key = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("OKX_PASSPHRASE") {
            if !v.trim().is_empty() { settings.okx.passphrase = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("OKX_BASE_URL") {
            if !v.trim().is_empty() { settings.okx.base_url = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("OKX_DEMO_TRADING") {
            settings.okx.demo_trading = v.trim().eq_ignore_ascii_case("true") || v.trim() == "1";
        }
        if let Ok(v) = std::env::var("OKX_AUTO_TRADING_ENABLED") {
            settings.okx.auto_trading_enabled = v.trim().eq_ignore_ascii_case("true") || v.trim() == "1";
        }
        if let Ok(v) = std::env::var("OKX_LIVE_TRADING_ACKNOWLEDGED") {
            settings.okx.live_trading_acknowledged = v.trim().eq_ignore_ascii_case("true") || v.trim() == "1";
        }
        if let Ok(v) = std::env::var("OKX_DEFAULT_ORDER_SIZE") {
            if let Ok(num) = v.trim().parse::<f64>() { settings.okx.default_order_size = num; }
        }
        if let Ok(v) = std::env::var("OKX_DEFAULT_LEVERAGE") {
            if let Ok(num) = v.trim().parse::<f64>() { settings.okx.default_leverage = num; }
        }
        if let Ok(v) = std::env::var("OKX_TRADE_MODE") {
            if !v.trim().is_empty() { settings.okx.trade_mode = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("OKX_POSITION_MODE") {
            if !v.trim().is_empty() { settings.okx.position_mode = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("OKX_BLOCK_NEW_ENTRIES_WHEN_POSITION_OPEN") {
            settings.okx.block_new_entries_when_position_open = v.trim().eq_ignore_ascii_case("true") || v.trim() == "1";
        }
        if let Ok(v) = std::env::var("OKX_MAX_SIGNAL_AGE_SECONDS") {
            if let Ok(num) = v.trim().parse::<u64>() { settings.okx.max_signal_age_seconds = num; }
        }
        if let Ok(v) = std::env::var("OKX_MAX_PENDING_BARS") {
            if let Ok(num) = v.trim().parse::<usize>() { settings.okx.max_pending_bars = num; }
        }
        if let Ok(v) = std::env::var("OKX_AUTOMATION_SESSION_PRESET") {
            if !v.trim().is_empty() { settings.okx.automation_session_preset = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("OKX_AUTOMATION_SESSION_TIMEZONE") {
            if !v.trim().is_empty() { settings.okx.automation_session_timezone = v.trim().to_string(); }
        }

        settings
    }

    pub fn is_provider_configured(&self) -> bool {
        !self.provider.api_key.trim().is_empty()
    }

    pub fn is_okx_configured(&self) -> bool {
        !self.okx.api_key.trim().is_empty()
            && !self.okx.secret_key.trim().is_empty()
            && !self.okx.passphrase.trim().is_empty()
    }
}
