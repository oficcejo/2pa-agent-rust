use crate::data::base::KlineBar;
use crate::learning::feedback::{ExitReason, TradeOutcome, PNL_SOURCE_MODEL};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Rejection reason emitted by a trading lifecycle hook.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HookRejection {
    CircuitBreakerTriggered(String),
    RiskLimitExceeded(String),
    PolicyViolation(String),
    Custom(String),
}

impl std::fmt::Display for HookRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CircuitBreakerTriggered(msg) => write!(f, "熔断触发: {}", msg),
            Self::RiskLimitExceeded(msg) => write!(f, "风控超限: {}", msg),
            Self::PolicyViolation(msg) => write!(f, "策略约束违规: {}", msg),
            Self::Custom(msg) => write!(f, "Hook 拦截: {}", msg),
        }
    }
}

impl std::error::Error for HookRejection {}

/// Action directed by a hook during pre-execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HookAction {
    /// Allow real execution to proceed normally.
    Proceed,
    /// Intercept and divert the order to shadow trading (virtual matching).
    InterceptShadow {
        shadow_order_id: String,
        simulated_entry_price: f64,
        note: String,
    },
    /// Skip execution deliberately (e.g. voluntary risk back-off).
    Skip(String),
}

/// Context supplied before running AI / strategy market analysis.
#[derive(Debug, Clone)]
pub struct PreAnalysisContext {
    pub symbol: String,
    pub timeframe: String,
    pub trading_system: String,
    pub timestamp_ms: i64,
    pub account_equity_usd: Option<f64>,
}

/// Context supplied before executing a generated decision order.
#[derive(Debug, Clone)]
pub struct PreExecutionContext {
    pub symbol: String,
    pub timeframe: String,
    pub trading_system: String,
    pub decision: serde_json::Value,
    pub account_balance_usd: f64,
    pub is_shadow_mode: bool,
}

/// Context supplied after an order outcome has been resolved.
#[derive(Debug, Clone)]
pub struct PostOutcomeContext {
    pub outcome: TradeOutcome,
}

/// Interceptor hook trait for the entire trading lifecycle.
pub trait TradingHook: Send + Sync {
    fn name(&self) -> &str;

    /// Called before analysis. Can halt or prevent analysis (e.g. daily drawdown circuit breaker).
    fn pre_analysis(&self, _ctx: &PreAnalysisContext) -> Result<(), HookRejection> {
        Ok(())
    }

    /// Called before order execution. Can intercept to shadow trading, modify, or block orders.
    fn pre_execution(&self, _ctx: &PreExecutionContext) -> Result<HookAction, HookRejection> {
        Ok(HookAction::Proceed)
    }

    /// Called after an outcome is resolved. Can record stats, update drawdown, or trigger reflection.
    fn post_outcome(&self, _ctx: &PostOutcomeContext) -> Result<(), HookRejection> {
        Ok(())
    }
}

/// Hook that monitors daily cumulative losses and triggers an emergency circuit breaker.
///
/// When cumulative daily loss breaches `max_loss_usd`, it halts further analysis
/// and blocks any new order execution until reset at the next trading day.
pub struct DailyDrawdownGuardHook {
    max_loss_usd: Arc<RwLock<f64>>,
    cumulative_loss_usd: Arc<RwLock<f64>>,
    tripped: Arc<RwLock<bool>>,
    day_timestamp_ms: Arc<RwLock<i64>>,
    timezone_offset_ms: Arc<RwLock<i64>>,
}

impl DailyDrawdownGuardHook {
    pub fn new(max_loss_usd: f64) -> Self {
        Self {
            max_loss_usd: Arc::new(RwLock::new(max_loss_usd.abs())),
            cumulative_loss_usd: Arc::new(RwLock::new(0.0)),
            tripped: Arc::new(RwLock::new(false)),
            day_timestamp_ms: Arc::new(RwLock::new(chrono::Utc::now().timestamp_millis())),
            timezone_offset_ms: Arc::new(RwLock::new(0)),
        }
    }

    pub fn max_loss_usd(&self) -> f64 {
        *self.max_loss_usd.read()
    }

    pub fn set_max_loss_usd(&self, limit: f64) {
        *self.max_loss_usd.write() = limit.abs();
    }

    pub fn set_timezone_offset_ms(&self, offset_ms: i64) {
        *self.timezone_offset_ms.write() = offset_ms;
    }

    pub fn current_drawdown_usd(&self) -> f64 {
        *self.cumulative_loss_usd.read()
    }

    pub fn is_tripped(&self) -> bool {
        *self.tripped.read()
    }

    pub fn reset(&self) {
        *self.cumulative_loss_usd.write() = 0.0;
        *self.tripped.write() = false;
        *self.day_timestamp_ms.write() = chrono::Utc::now().timestamp_millis();
    }

    pub fn check_day_rollover(&self) {
        let offset = *self.timezone_offset_ms.read();
        let now_ms = chrono::Utc::now().timestamp_millis() + offset;
        let last_day = (*self.day_timestamp_ms.read() + offset) / 86_400_000;
        let current_day = now_ms / 86_400_000;
        if current_day != last_day {
            self.reset();
        }
    }

    pub fn record_loss(&self, loss_usd: f64) {
        self.check_day_rollover();
        if loss_usd > 0.0 && loss_usd.is_finite() {
            let mut cum = self.cumulative_loss_usd.write();
            *cum += loss_usd;
            if *cum >= *self.max_loss_usd.read() {
                *self.tripped.write() = true;
            }
        }
    }
}

impl TradingHook for DailyDrawdownGuardHook {
    fn name(&self) -> &str {
        "daily_drawdown_guard"
    }

    fn pre_analysis(&self, _ctx: &PreAnalysisContext) -> Result<(), HookRejection> {
        self.check_day_rollover();
        if *self.tripped.read() {
            return Err(HookRejection::CircuitBreakerTriggered(format!(
                "单日累计亏损 ${:.2} 已达熔断阈值 ${:.2}，已暂停分析",
                *self.cumulative_loss_usd.read(),
                *self.max_loss_usd.read()
            )));
        }
        Ok(())
    }

    fn pre_execution(&self, _ctx: &PreExecutionContext) -> Result<HookAction, HookRejection> {
        self.check_day_rollover();
        if *self.tripped.read() {
            return Err(HookRejection::CircuitBreakerTriggered(format!(
                "单日累计亏损 ${:.2} 已超熔断阈值 ${:.2}，禁止新开仓",
                *self.cumulative_loss_usd.read(),
                *self.max_loss_usd.read()
            )));
        }
        Ok(HookAction::Proceed)
    }

    fn post_outcome(&self, ctx: &PostOutcomeContext) -> Result<(), HookRejection> {
        if ctx.outcome.realized_pnl_usd < 0.0 {
            self.record_loss(-ctx.outcome.realized_pnl_usd);
        }
        Ok(())
    }
}

/// Simulated shadow position for paper trading without venue execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowPosition {
    pub shadow_id: String,
    pub symbol: String,
    pub side: String,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub entry_ts: i64,
    pub hold_bars: u32,
    pub max_favorable_px: f64,
    pub max_adverse_px: f64,
    pub active: bool,
}

/// Hook that intercepts candidate or testing orders and routes them to virtual fill matching.
pub struct ShadowTradingHook {
    active_positions: Arc<RwLock<Vec<ShadowPosition>>>,
    closed_outcomes: Arc<RwLock<Vec<TradeOutcome>>>,
}

impl Default for ShadowTradingHook {
    fn default() -> Self {
        Self::new()
    }
}

impl ShadowTradingHook {
    pub fn new() -> Self {
        Self {
            active_positions: Arc::new(RwLock::new(Vec::new())),
            closed_outcomes: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn active_positions(&self) -> Vec<ShadowPosition> {
        self.active_positions.read().clone()
    }

    pub fn closed_outcomes(&self) -> Vec<TradeOutcome> {
        self.closed_outcomes.read().clone()
    }

    /// Match open shadow positions against incoming market bars.
    pub fn match_bar(&self, symbol: &str, bar: &KlineBar) -> Vec<TradeOutcome> {
        let mut resolved = Vec::new();
        let mut positions = self.active_positions.write();

        for pos in positions.iter_mut().filter(|p| p.active && p.symbol == symbol) {
            pos.hold_bars += 1;
            let is_long = pos.side == "long";

            if is_long {
                pos.max_favorable_px = pos.max_favorable_px.max(bar.high);
                pos.max_adverse_px = pos.max_adverse_px.min(bar.low);
            } else {
                pos.max_favorable_px = pos.max_favorable_px.min(bar.low);
                pos.max_adverse_px = pos.max_adverse_px.max(bar.high);
            }

            let risk = (pos.entry_price - pos.stop_loss).abs().max(0.001);
            let mut exit_price = None;
            let mut exit_reason = None;

            if is_long {
                if bar.low <= pos.stop_loss {
                    exit_price = Some(pos.stop_loss);
                    exit_reason = Some(ExitReason::StopLoss);
                } else if bar.high >= pos.take_profit {
                    exit_price = Some(pos.take_profit);
                    exit_reason = Some(ExitReason::TakeProfit);
                }
            } else if bar.high >= pos.stop_loss {
                exit_price = Some(pos.stop_loss);
                exit_reason = Some(ExitReason::StopLoss);
            } else if bar.low <= pos.take_profit {
                exit_price = Some(pos.take_profit);
                exit_reason = Some(ExitReason::TakeProfit);
            }

            if let (Some(px), Some(reason)) = (exit_price, exit_reason) {
                pos.active = false;
                let sign = if is_long { 1.0 } else { -1.0 };
                let r_mult = (px - pos.entry_price) * sign / risk;
                let mfe_r = if is_long {
                    (pos.max_favorable_px - pos.entry_price) / risk
                } else {
                    (pos.entry_price - pos.max_favorable_px) / risk
                };
                let mae_r = if is_long {
                    (pos.entry_price - pos.max_adverse_px) / risk
                } else {
                    (pos.max_adverse_px - pos.entry_price) / risk
                };

                let outcome = TradeOutcome {
                    signal_id: format!("shadow_sig_{}", pos.shadow_id),
                    decision_record_id: format!("shadow_rec_{}", pos.shadow_id),
                    strategy_id: "shadow".to_string(),
                    strategy_version: "shadow_v1".to_string(),
                    prompt_version: "shadow_candidate".to_string(),
                    prompt_hash: "shadow".to_string(),
                    symbol: pos.symbol.clone(),
                    timeframe: "15m".to_string(),
                    side: pos.side.clone(),
                    cycle_position: "shadow".to_string(),
                    detected_patterns: vec!["shadow_virtual".to_string()],
                    entry_price: pos.entry_price,
                    stop_price: pos.stop_loss,
                    target_price: pos.take_profit,
                    exit_price: Some(px),
                    size: 1.0,
                    filled: true,
                    fill_ratio: 1.0,
                    fees_usd: 1.0,
                    realized_pnl_usd: r_mult * 100.0,
                    pnl_source: PNL_SOURCE_MODEL.to_string(),
                    r_multiple: r_mult,
                    mfe_r: mfe_r.max(0.0),
                    mae_r: mae_r.max(0.0),
                    hold_bars: pos.hold_bars,
                    exit_reason: reason.as_str().to_string(),
                    qualified: true,
                    qualification_reason: "影子交易虚拟撮合完成".to_string(),
                    created_ms: pos.entry_ts,
                    resolved_ms: bar.ts_open,
                };
                resolved.push(outcome);
            }
        }

        // Clean up resolved positions
        positions.retain(|p| p.active);
        self.closed_outcomes.write().extend(resolved.clone());
        resolved
    }
}

impl TradingHook for ShadowTradingHook {
    fn name(&self) -> &str {
        "shadow_trading"
    }

    fn pre_execution(&self, ctx: &PreExecutionContext) -> Result<HookAction, HookRejection> {
        if ctx.is_shadow_mode {
            let shadow_id = uuid::Uuid::new_v4().simple().to_string();
            let parse_px = |v: Option<&serde_json::Value>| {
                v.and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse::<f64>().ok())))
            };
            let entry_px = parse_px(ctx.decision.get("entry_price")).unwrap_or(100.0);
            let side = ctx
                .decision
                .get("order_direction")
                .and_then(|v| v.as_str())
                .map(|s| {
                    let lower = s.to_lowercase();
                    if lower.contains("空") || lower.contains("short") || lower.contains("sell") {
                        "short"
                    } else {
                        "long"
                    }
                })
                .unwrap_or("long");
            let is_short = side == "short";
            let default_sl = if is_short { entry_px * 1.01 } else { entry_px * 0.99 };
            let default_tp = if is_short { entry_px * 0.98 } else { entry_px * 1.02 };
            let sl_px = parse_px(ctx.decision.get("stop_loss_price")).unwrap_or(default_sl);
            let tp_px = parse_px(ctx.decision.get("take_profit_price")).unwrap_or(default_tp);

            self.active_positions.write().push(ShadowPosition {
                shadow_id: shadow_id.clone(),
                symbol: ctx.symbol.clone(),
                side: side.to_string(),
                entry_price: entry_px,
                stop_loss: sl_px,
                take_profit: tp_px,
                entry_ts: chrono::Utc::now().timestamp_millis(),
                hold_bars: 0,
                max_favorable_px: entry_px,
                max_adverse_px: entry_px,
                active: true,
            });

            return Ok(HookAction::InterceptShadow {
                shadow_order_id: shadow_id,
                simulated_entry_price: entry_px,
                note: "订单已拦截进入影子撮合通道".to_string(),
            });
        }
        Ok(HookAction::Proceed)
    }
}

/// Hook that verifies whether a trading decision meets the TypeSafe calibrated confidence threshold.
pub struct TypeSafeConfidenceGuardHook {
    pub min_confidence: Arc<RwLock<f64>>,
}

impl TypeSafeConfidenceGuardHook {
    pub fn new(min_confidence: f64) -> Self {
        Self {
            min_confidence: Arc::new(RwLock::new(min_confidence)),
        }
    }

    pub fn set_min_confidence(&self, val: f64) {
        *self.min_confidence.write() = val;
    }
}

impl TradingHook for TypeSafeConfidenceGuardHook {
    fn name(&self) -> &str {
        "TypeSafeConfidenceGuardHook"
    }

    fn pre_execution(&self, ctx: &PreExecutionContext) -> Result<HookAction, HookRejection> {
        let action = ctx.decision.get("action").and_then(|v| v.as_str()).unwrap_or("WAIT");
        if action != "OPEN" {
            return Ok(HookAction::Proceed);
        }

        let min_conf = *self.min_confidence.read();
        let typesafe_conf = ctx
            .decision
            .get("typesafe_confidence")
            .and_then(|v| v.as_f64())
            .or_else(|| {
                ctx.decision
                    .get("stage1_diagnosis")
                    .and_then(|s1| s1.get("typesafe_confidence"))
                    .and_then(|v| v.as_f64())
            });

        if let Some(conf) = typesafe_conf {
            if conf < min_conf {
                return Err(HookRejection::RiskLimitExceeded(format!(
                    "TypeSafe 数学校准置信度 ({:.2}) 低于安全入场门槛 ({:.2})，拒绝开仓",
                    conf, min_conf
                )));
            }
        }

        Ok(HookAction::Proceed)
    }
}

/// Ordered lifecycle hook execution pipeline.
pub struct HookPipeline {
    hooks: Vec<Arc<dyn TradingHook>>,
}

impl Default for HookPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl HookPipeline {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    pub fn add_hook(&mut self, hook: Arc<dyn TradingHook>) {
        self.hooks.push(hook);
    }

    pub fn run_pre_analysis(&self, ctx: &PreAnalysisContext) -> Result<(), HookRejection> {
        for hook in &self.hooks {
            hook.pre_analysis(ctx)?;
        }
        Ok(())
    }

    pub fn run_pre_execution(&self, ctx: &PreExecutionContext) -> Result<HookAction, HookRejection> {
        for hook in &self.hooks {
            match hook.pre_execution(ctx)? {
                HookAction::Proceed => {}
                intercept => return Ok(intercept),
            }
        }
        Ok(HookAction::Proceed)
    }

    pub fn run_post_outcome(&self, ctx: &PostOutcomeContext) -> Result<(), HookRejection> {
        for hook in &self.hooks {
            hook.post_outcome(ctx)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daily_drawdown_guard_trips_on_limit() {
        let guard = DailyDrawdownGuardHook::new(300.0);
        assert!(!guard.is_tripped());

        // Record loss under limit
        guard.record_loss(150.0);
        assert!(!guard.is_tripped());

        let ctx = PreAnalysisContext {
            symbol: "BTC-USDT-SWAP".to_string(),
            timeframe: "15m".to_string(),
            trading_system: "2pa".to_string(),
            timestamp_ms: 1000,
            account_equity_usd: Some(10000.0),
        };
        assert!(guard.pre_analysis(&ctx).is_ok());

        // Breach limit
        guard.record_loss(160.0);
        assert!(guard.is_tripped());

        let result = guard.pre_analysis(&ctx);
        assert!(matches!(result, Err(HookRejection::CircuitBreakerTriggered(_))));

        // Reset clears breaker
        guard.reset();
        assert!(!guard.is_tripped());
        assert!(guard.pre_analysis(&ctx).is_ok());
    }

    #[test]
    fn test_shadow_trading_hook_intercepts_and_matches() {
        let shadow = Arc::new(ShadowTradingHook::new());
        let mut pipeline = HookPipeline::new();
        pipeline.add_hook(shadow.clone());

        let ctx = PreExecutionContext {
            symbol: "BTC-USDT-SWAP".to_string(),
            timeframe: "15m".to_string(),
            trading_system: "2pa".to_string(),
            decision: serde_json::json!({
                "action": "OPEN",
                "order_direction": "做多",
                "entry_price": 50000.0,
                "stop_loss_price": 49500.0,
                "take_profit_price": 51000.0
            }),
            account_balance_usd: 10000.0,
            is_shadow_mode: true,
        };

        let action = pipeline.run_pre_execution(&ctx).expect("pre_execution");
        assert!(matches!(action, HookAction::InterceptShadow { .. }));
        assert_eq!(shadow.active_positions().len(), 1);

        // Bar reaches take profit
        let bar = KlineBar {
            seq: 1,
            ts_open: 2000,
            open: 50100.0,
            high: 51200.0,
            low: 49900.0,
            close: 51100.0,
            volume: 10.0,
            amount: 0.0,
            pct_chg: None,
            closed: true,
        };

        let resolved = shadow.match_bar("BTC-USDT-SWAP", &bar);
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].r_multiple >= 2.0);
        assert_eq!(resolved[0].exit_reason, "take_profit");
        assert_eq!(shadow.active_positions().len(), 0);
    }

    #[test]
    fn test_typesafe_confidence_guard_hook() {
        let guard = Arc::new(TypeSafeConfidenceGuardHook::new(0.70));
        let mut pipeline = HookPipeline::new();
        pipeline.add_hook(guard.clone());

        // 1. High confidence decision passes
        let ctx_pass = PreExecutionContext {
            symbol: "ETH-USDT-SWAP".to_string(),
            timeframe: "15m".to_string(),
            trading_system: "2pa_trend".to_string(),
            decision: serde_json::json!({
                "action": "OPEN",
                "typesafe_confidence": 0.85
            }),
            account_balance_usd: 10000.0,
            is_shadow_mode: false,
        };
        assert!(pipeline.run_pre_execution(&ctx_pass).is_ok());

        // 2. Low confidence decision rejected
        let ctx_reject = PreExecutionContext {
            symbol: "ETH-USDT-SWAP".to_string(),
            timeframe: "15m".to_string(),
            trading_system: "2pa_trend".to_string(),
            decision: serde_json::json!({
                "action": "OPEN",
                "typesafe_confidence": 0.45
            }),
            account_balance_usd: 10000.0,
            is_shadow_mode: false,
        };
        let res = pipeline.run_pre_execution(&ctx_reject);
        assert!(matches!(res, Err(HookRejection::RiskLimitExceeded(_))));

        // 3. Dynamic adjustment of threshold
        guard.set_min_confidence(0.40);
        assert!(pipeline.run_pre_execution(&ctx_reject).is_ok());
    }
}
