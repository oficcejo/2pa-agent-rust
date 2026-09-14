//! Structured trade feedback.
//!
//! A scalar win/loss label is not enough to learn from. This module turns a
//! closed trade into a structured signal: realised R multiple, maximum
//! favourable/adverse excursion (MFE/MAE), holding time, fees and an exit
//! reason. These are the fields the experience library consumes, and the
//! fields the candidate evaluator compares.
//!
//! All functions here are pure so they can be unit tested without a network
//! or filesystem.

use serde::{Deserialize, Serialize};

/// PnL reconciled from venue fill data.
pub const PNL_SOURCE_BROKER: &str = "broker";
/// PnL derived from price movement and position size, not from fills.
pub const PNL_SOURCE_MODEL: &str = "model_estimate";

/// Direction normalised to `long` / `short`.
pub fn normalize_side(raw: &str) -> Option<&'static str> {
    match raw.trim().to_lowercase().as_str() {
        "long" | "做多" | "buy" | "多" => Some("long"),
        "short" | "做空" | "sell" | "空" => Some("short"),
        _ => None,
    }
}

/// Direction sign: +1 for long, -1 for short.
pub fn side_sign(side: &str) -> Option<f64> {
    normalize_side(side).map(|s| if s == "long" { 1.0 } else { -1.0 })
}

/// One bar reduced to the fields that matter for excursion analysis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarHlc {
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

impl BarHlc {
    pub fn new(high: f64, low: f64, close: f64) -> Self {
        Self { high, low, close }
    }
}

/// Excursion summary measured in R (multiples of the initial risk).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Excursion {
    /// Best unrealised profit reached, in R. Never negative.
    pub mfe_r: f64,
    /// Worst unrealised drawdown reached, in R. Reported as a positive number.
    pub mae_r: f64,
    /// Number of bars inspected before the trade resolved.
    pub hold_bars: u32,
}

/// Realised R multiple of a closed trade.
///
/// Risk is fixed at entry as `|entry - stop|`, which is the definition the
/// strategy protocol already uses for its net reward gate. Returns `None` for
/// degenerate inputs (zero risk, non-finite prices) instead of guessing.
pub fn realized_r(side: &str, entry: f64, stop: f64, exit: f64) -> Option<f64> {
    let sign = side_sign(side)?;
    if !entry.is_finite() || !stop.is_finite() || !exit.is_finite() {
        return None;
    }
    let risk = (entry - stop).abs();
    if risk <= 0.0 {
        return None;
    }
    Some((exit - entry) * sign / risk)
}

/// Walk forward over the bars that followed the entry and measure how far the
/// trade ran for and against the position before it resolved.
///
/// `bars` must be ordered oldest to newest and contain only bars at or after
/// the entry bar. `stop` and `target` terminate the walk: the first bar whose
/// range touches either level ends the excursion.
pub fn excursion(
    side: &str,
    entry: f64,
    stop: f64,
    target: Option<f64>,
    bars: &[BarHlc],
) -> Option<Excursion> {
    let sign = side_sign(side)?;
    if !entry.is_finite() || !stop.is_finite() {
        return None;
    }
    let risk = (entry - stop).abs();
    if risk <= 0.0 {
        return None;
    }

    let mut mfe = 0.0_f64;
    let mut mae = 0.0_f64;
    let mut hold_bars = 0_u32;

    for (idx, bar) in bars.iter().enumerate() {
        if !bar.high.is_finite() || !bar.low.is_finite() {
            continue;
        }
        let best = if sign > 0.0 { bar.high } else { bar.low };
        let worst = if sign > 0.0 { bar.low } else { bar.high };
        mfe = mfe.max((best - entry) * sign / risk);
        mae = mae.max((entry - worst) * sign / risk);
        hold_bars = (idx + 1) as u32;

        // Stop is checked first: intra-bar sequencing is unknown, so the
        // pessimistic assumption is taken, matching the risk-first posture of
        // the rest of the project.
        let stop_hit = if sign > 0.0 { bar.low <= stop } else { bar.high >= stop };
        let target_hit = target
            .filter(|t| t.is_finite())
            .map(|t| if sign > 0.0 { bar.high >= t } else { bar.low <= t })
            .unwrap_or(false);
        if stop_hit || target_hit {
            break;
        }
    }

    Some(Excursion { mfe_r: mfe.max(0.0), mae_r: mae.max(0.0), hold_bars })
}

/// Exit classification for a resolved trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    TakeProfit,
    StopLoss,
    ClosedEarly,
    Expired,
    /// Entry order never filled; the decision produced no position.
    Unfilled,
}

impl ExitReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExitReason::TakeProfit => "take_profit",
            ExitReason::StopLoss => "stop_loss",
            ExitReason::ClosedEarly => "closed_early",
            ExitReason::Expired => "expired",
            ExitReason::Unfilled => "unfilled",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_lowercase().as_str() {
            "take_profit" | "tp" => Some(ExitReason::TakeProfit),
            "stop_loss" | "sl" => Some(ExitReason::StopLoss),
            "closed_early" | "close_early" => Some(ExitReason::ClosedEarly),
            "expired" => Some(ExitReason::Expired),
            "unfilled" => Some(ExitReason::Unfilled),
            _ => None,
        }
    }
}

/// A fully resolved decision → execution → outcome record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeOutcome {
    /// Order receipt id; ties an outcome back to its decision and audit row.
    pub signal_id: String,
    /// Analysis record that produced the decision, when known.
    #[serde(default)]
    pub decision_record_id: String,
    #[serde(default)]
    pub strategy_id: String,
    #[serde(default)]
    pub strategy_version: String,
    /// Version + hash of the prompt artifact that produced the decision.
    #[serde(default)]
    pub prompt_version: String,
    #[serde(default)]
    pub prompt_hash: String,
    pub symbol: String,
    pub timeframe: String,
    /// `long` or `short`.
    pub side: String,
    #[serde(default)]
    pub cycle_position: String,
    #[serde(default)]
    pub detected_patterns: Vec<String>,
    pub entry_price: f64,
    pub stop_price: f64,
    pub target_price: f64,
    pub exit_price: Option<f64>,
    #[serde(default)]
    pub size: f64,
    /// Whether the entry order actually filled.
    pub filled: bool,
    /// Filled fraction of the submitted size, 0..=1.
    #[serde(default)]
    pub fill_ratio: f64,
    #[serde(default)]
    pub fees_usd: f64,
    #[serde(default)]
    pub realized_pnl_usd: f64,
    /// Provenance of `realized_pnl_usd`. Never present an estimate as if it
    /// were a reconciled venue figure.
    #[serde(default)]
    pub pnl_source: String,
    /// Realised result expressed in R.
    #[serde(default)]
    pub r_multiple: f64,
    #[serde(default)]
    pub mfe_r: f64,
    #[serde(default)]
    pub mae_r: f64,
    #[serde(default)]
    pub hold_bars: u32,
    #[serde(default)]
    pub exit_reason: String,
    /// Whether this sample is clean enough to enter the experience library.
    #[serde(default)]
    pub qualified: bool,
    #[serde(default)]
    pub qualification_reason: String,
    pub created_ms: i64,
    #[serde(default)]
    pub resolved_ms: i64,
}

impl TradeOutcome {
    /// A filled, profitable trade.
    pub fn is_win(&self) -> bool {
        self.filled && self.r_multiple > 0.0
    }
}

/// Rules deciding which resolved trades are trustworthy enough to learn from.
///
/// Deliberately conservative: a trade only becomes training material when the
/// order actually filled, the outcome is not a pathological outlier, and the
/// position was held long enough for the entry logic to be evaluated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualificationPolicy {
    pub require_filled: bool,
    pub min_hold_bars: u32,
    pub max_abs_r: f64,
}

impl Default for QualificationPolicy {
    fn default() -> Self {
        Self { require_filled: true, min_hold_bars: 1, max_abs_r: 25.0 }
    }
}

/// Apply the policy and explain the verdict.
///
/// Returns `(qualified, reason)`. The reason is always populated so it can be
/// stored verbatim next to the outcome for later auditing.
pub fn qualify(outcome: &TradeOutcome, policy: &QualificationPolicy) -> (bool, String) {
    if policy.require_filled && !outcome.filled {
        return (false, "入场单未成交，不构成可学习样本".to_string());
    }
    if outcome.entry_price <= 0.0 || outcome.stop_price <= 0.0 {
        return (false, "入场价或止损价无效".to_string());
    }
    if (outcome.entry_price - outcome.stop_price).abs() <= 0.0 {
        return (false, "初始风险为零，R 倍数无定义".to_string());
    }
    if outcome.hold_bars < policy.min_hold_bars {
        return (
            false,
            format!("持仓仅 {} 根 K 线，短于资格下限 {}", outcome.hold_bars, policy.min_hold_bars),
        );
    }
    if !outcome.r_multiple.is_finite() || outcome.r_multiple.abs() > policy.max_abs_r {
        return (
            false,
            format!("R 倍数 {} 超出合理范围（疑似价格或数量异常）", outcome.r_multiple),
        );
    }
    if let ExitReason::Unfilled = ExitReason::parse(&outcome.exit_reason).unwrap_or(ExitReason::Unfilled) {
        return (false, "未成交出场，样本不完整".to_string());
    }
    (true, "满足经验库准入条件".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(r: f64, filled: bool, hold: u32) -> TradeOutcome {
        TradeOutcome {
            signal_id: "s".into(),
            decision_record_id: "d".into(),
            strategy_id: "2pa_trend".into(),
            strategy_version: "2026-09-v1".into(),
            prompt_version: "v1".into(),
            prompt_hash: "abc".into(),
            symbol: "BTC-USDT-SWAP".into(),
            timeframe: "15m".into(),
            side: "long".into(),
            cycle_position: "trending_tr".into(),
            detected_patterns: vec!["h2".into()],
            entry_price: 100.0,
            stop_price: 99.0,
            target_price: 104.0,
            exit_price: Some(104.0),
            size: 1.0,
            filled,
            fill_ratio: 1.0,
            fees_usd: 0.1,
            realized_pnl_usd: 4.0,
            pnl_source: PNL_SOURCE_MODEL.into(),
            r_multiple: r,
            mfe_r: 4.0,
            mae_r: 0.2,
            hold_bars: hold,
            exit_reason: "take_profit".into(),
            qualified: false,
            qualification_reason: String::new(),
            created_ms: 0,
            resolved_ms: 0,
        }
    }

    #[test]
    fn side_normalisation_accepts_chinese() {
        assert_eq!(normalize_side("做多"), Some("long"));
        assert_eq!(normalize_side("做空"), Some("short"));
        assert_eq!(normalize_side("nonsense"), None);
    }

    #[test]
    fn realized_r_is_direction_aware() {
        // Long: +2R when price moves two risk units up.
        assert_eq!(realized_r("long", 100.0, 99.0, 102.0), Some(2.0));
        // Short: +2R when price moves two risk units down.
        assert_eq!(realized_r("short", 100.0, 101.0, 98.0), Some(2.0));
        // Stop out is exactly -1R on both sides.
        assert_eq!(realized_r("long", 100.0, 99.0, 99.0), Some(-1.0));
        assert_eq!(realized_r("short", 100.0, 101.0, 101.0), Some(-1.0));
    }

    #[test]
    fn realized_r_rejects_degenerate_risk() {
        assert_eq!(realized_r("long", 100.0, 100.0, 105.0), None);
        assert_eq!(realized_r("long", f64::NAN, 99.0, 105.0), None);
    }

    #[test]
    fn excursion_tracks_mfe_and_mae_until_stop() {
        let bars = [
            BarHlc::new(101.0, 99.5, 100.5), // +1R high, -0.5R low
            BarHlc::new(103.0, 99.2, 102.0), // +3R high, -0.8R low
            BarHlc::new(105.0, 98.5, 104.0), // +5R high, -1.5R low, stop touched
        ];
        let e = excursion("long", 100.0, 99.0, None, &bars).unwrap();
        assert_eq!(e.mfe_r, 5.0);
        assert_eq!(e.mae_r, 1.5);
        assert_eq!(e.hold_bars, 3);
    }

    #[test]
    fn excursion_stops_at_first_stop_touch() {
        let bars = [
            BarHlc::new(100.5, 99.5, 100.0), // inside range
            BarHlc::new(100.0, 98.0, 98.5),  // stop touched, sets -2R MAE
            BarHlc::new(200.0, 98.0, 199.0), // must be ignored
        ];
        let e = excursion("long", 100.0, 99.0, None, &bars).unwrap();
        assert_eq!(e.hold_bars, 2);
        assert_eq!(e.mae_r, 2.0);
        assert_eq!(e.mfe_r, 0.5);
    }

    #[test]
    fn excursion_stops_at_target() {
        let bars = [
            BarHlc::new(101.0, 99.5, 100.0),
            BarHlc::new(105.0, 99.5, 104.5), // target 104 hit
            BarHlc::new(120.0, 99.5, 119.0),  // ignored
        ];
        let e = excursion("long", 100.0, 99.0, Some(104.0), &bars).unwrap();
        assert_eq!(e.hold_bars, 2);
        assert_eq!(e.mfe_r, 5.0);
    }

    #[test]
    fn short_excursion_is_mirrored() {
        let bars = [
            BarHlc::new(100.5, 99.0, 99.5),
            BarHlc::new(101.5, 96.0, 97.0), // +4R low, -1.5R high
        ];
        let e = excursion("short", 100.0, 101.0, None, &bars).unwrap();
        assert_eq!(e.mfe_r, 4.0);
        assert_eq!(e.mae_r, 1.5);
    }

    #[test]
    fn qualification_rejects_unfilled_and_short_holds() {
        let policy = QualificationPolicy { require_filled: true, min_hold_bars: 3, max_abs_r: 25.0 };
        let (ok, why) = qualify(&outcome(2.0, false, 10), &policy);
        assert!(!ok);
        assert!(why.contains("未成交"));

        let (ok, why) = qualify(&outcome(2.0, true, 1), &policy);
        assert!(!ok);
        assert!(why.contains("短于资格下限"));

        let (ok, why) = qualify(&outcome(2.0, true, 5), &policy);
        assert!(ok, "{why}");
    }

    #[test]
    fn qualification_rejects_outlier_r() {
        let policy = QualificationPolicy::default();
        let (ok, why) = qualify(&outcome(900.0, true, 5), &policy);
        assert!(!ok);
        assert!(why.contains("超出合理范围"));
    }

    #[test]
    fn qualification_rejects_unfilled_exit_reason() {
        let policy = QualificationPolicy { require_filled: false, min_hold_bars: 0, max_abs_r: 25.0 };
        let mut o = outcome(0.0, true, 5);
        o.exit_reason = "unfilled".into();
        let (ok, _) = qualify(&o, &policy);
        assert!(!ok);
    }
}
