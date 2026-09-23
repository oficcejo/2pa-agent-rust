//! Core data structures and types for the backtesting engine.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum BacktestDataSource {
    #[default]
    #[serde(rename = "okx_api", alias = "okx_history", alias = "OkxHistory", alias = "okx")]
    OkxApi,
    #[serde(rename = "local_file", alias = "LocalFile", alias = "file")]
    LocalFile,
    #[serde(rename = "synthetic", alias = "Synthetic", alias = "simulated")]
    Synthetic,
}

/// User-supplied configuration for running a backtest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestConfig {
    #[serde(default = "default_symbol")]
    pub symbol: String,

    #[serde(default = "default_timeframe")]
    pub timeframe: String,

    #[serde(default)]
    pub htf_timeframe: Option<String>,

    #[serde(default = "default_strategy")]
    pub strategy_id: String,

    pub start_time_ms: Option<i64>,
    pub end_time_ms: Option<i64>,

    #[serde(default = "default_initial_capital")]
    pub initial_capital: f64,

    #[serde(default = "default_risk_percent")]
    pub risk_percent: f64,

    #[serde(default = "default_max_margin_percent")]
    pub max_margin_percent: f64,

    #[serde(default = "default_leverage")]
    pub leverage: f64,

    #[serde(default = "default_taker_fee_rate")]
    pub taker_fee_rate: f64,

    #[serde(default = "default_maker_fee_rate")]
    pub maker_fee_rate: f64,

    #[serde(default = "default_slippage_rate")]
    pub slippage_rate: f64,

    #[serde(default = "default_true")]
    pub use_mechanical_exit: bool,

    #[serde(default = "default_true")]
    pub use_cache: bool,

    #[serde(default = "default_max_hold_bars")]
    pub max_hold_bars: u32,

    #[serde(default = "default_ct_val")]
    pub ct_val: f64,

    #[serde(default = "default_lot_sz")]
    pub lot_sz: f64,

    #[serde(default)]
    pub data_source: BacktestDataSource,

    pub fixture_path: Option<String>,

    /// Maximum total bars to process (safety cap, default 2000)
    #[serde(default = "default_max_bars")]
    pub max_bars: usize,

    /// Whether to invoke live LLM when diagnostics pass and cache misses.
    /// If false, uses deterministic diagnostic rule execution (zero LLM token cost).
    #[serde(default = "default_true")]
    pub allow_llm_calls: bool,

    /// Whether to use TypeSafe System One (Jev) model for fast calibrated candidate evaluations.
    #[serde(default)]
    pub use_typesafe: bool,
}

fn default_symbol() -> String { "BTC-USDT-SWAP".to_string() }
fn default_timeframe() -> String { "15m".to_string() }
fn default_strategy() -> String { "2pa_trend".to_string() }
fn default_initial_capital() -> f64 { 10000.0 }
fn default_risk_percent() -> f64 { 1.0 } // 1.0%
fn default_max_margin_percent() -> f64 { 50.0 } // 50%
fn default_leverage() -> f64 { 5.0 }
fn default_taker_fee_rate() -> f64 { 0.0005 } // 0.05%
fn default_maker_fee_rate() -> f64 { 0.0002 } // 0.02%
fn default_slippage_rate() -> f64 { 0.0002 }  // 0.02%
fn default_true() -> bool { true }
fn default_max_hold_bars() -> u32 { 48 }
fn default_ct_val() -> f64 { 0.01 }
fn default_lot_sz() -> f64 { 1.0 }
fn default_max_bars() -> usize { 2000 }

impl Default for BacktestConfig {
    fn default() -> Self {
        Self {
            symbol: default_symbol(),
            timeframe: default_timeframe(),
            htf_timeframe: Some("1H".to_string()),
            strategy_id: default_strategy(),
            start_time_ms: None,
            end_time_ms: None,
            initial_capital: default_initial_capital(),
            risk_percent: default_risk_percent(),
            max_margin_percent: default_max_margin_percent(),
            leverage: default_leverage(),
            taker_fee_rate: default_taker_fee_rate(),
            maker_fee_rate: default_maker_fee_rate(),
            slippage_rate: default_slippage_rate(),
            use_mechanical_exit: true,
            use_cache: true,
            max_hold_bars: default_max_hold_bars(),
            ct_val: default_ct_val(),
            lot_sz: default_lot_sz(),
            data_source: BacktestDataSource::default(),
            fixture_path: None,
            max_bars: default_max_bars(),
            allow_llm_calls: true,
            use_typesafe: false,
        }
    }
}

/// A single executed and closed trade in the backtest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestTrade {
    pub trade_id: String,
    pub signal_id: String,
    pub strategy_id: String,
    pub direction: String,         // "做多" or "做空"
    pub order_type: String,        // "市价单", "限价单", "突破单"
    pub entry_time_ms: i64,
    pub entry_price: f64,
    pub exit_time_ms: i64,
    pub exit_price: f64,
    pub contracts: f64,
    pub notional_usdt: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub gross_pnl: f64,
    pub net_pnl: f64,
    pub pnl_percent: f64,
    pub pnl_r: f64,
    pub exit_reason: String,       // "take_profit", "stop_loss", "expired", "liquidated", etc.
    pub fees: f64,
    pub slippage: f64,
    pub mfe_r: f64,
    pub mae_r: f64,
    pub hold_bars: u32,
    pub notes: String,
}

/// Snapshot of virtual account equity at each bar close.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquityPoint {
    pub timestamp_ms: i64,
    pub equity: f64,
    pub cash: f64,
    pub unrealized_pnl: f64,
    pub drawdown_pct: f64,
    pub in_position: bool,
}

/// Summary statistical metrics computed across the backtest.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BacktestMetrics {
    pub initial_capital: f64,
    pub final_equity: f64,
    pub net_profit: f64,
    pub net_profit_pct: f64,
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub break_even_trades: usize,
    pub win_rate: f64,             // e.g. 55.5 (%)
    pub profit_factor: f64,        // gross wins / gross losses
    pub expectancy_r: f64,         // average R
    pub max_drawdown_amount: f64,  // in USDT
    pub max_drawdown_pct: f64,     // e.g. 8.4 (%)
    pub sharpe_ratio: f64,         // annualized
    pub sortino_ratio: f64,        // annualized downside
    pub avg_trade_pnl: f64,
    pub avg_hold_bars: f64,
    pub max_consecutive_wins: usize,
    pub max_consecutive_losses: usize,
    pub total_fees_paid: f64,
    pub total_slippage_paid: f64,
    pub total_bars_processed: usize,
    pub gated_bars_skipped: usize, // bars short-circuited by diagnostics
    pub llm_evaluations: usize,    // actual LLM calls made
    pub cache_hits: usize,         // decisions retrieved from cache
}

/// Complete report containing config, metrics, equity curve, and trades.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestReport {
    pub job_id: String,
    pub config: BacktestConfig,
    pub metrics: BacktestMetrics,
    pub equity_curve: Vec<EquityPoint>,
    pub trades: Vec<BacktestTrade>,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
    pub execution_duration_ms: u64,
}

/// Progress / Status of a backtesting job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestJobStatus {
    pub job_id: String,
    pub status: String,            // "running", "completed", "failed"
    pub progress_pct: f64,         // 0.0 - 100.0
    pub current_bar: usize,
    pub total_bars: usize,
    pub message: String,
    pub error: Option<String>,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
}
