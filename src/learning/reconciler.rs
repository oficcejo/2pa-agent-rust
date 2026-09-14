//! The receipt → report loop.
//!
//! Reef links an agent response to its later feedback with a receipt id. The
//! equivalent here is `signal_id`: the executor derives it from the decision,
//! the audit log stores it, and the venue keeps it as part of the client order
//! id. That makes it possible to walk back from "we submitted this" to "here is
//! what actually happened" and finally to "here is the loss/profit in R".
//!
//! Before this module the loop stopped at `submitted: true`: the audit row
//! recorded that an order left the building, never what it did. Everything
//! downstream (experience library, candidate evaluation) was therefore starved
//! of data.

use crate::learning::experience_writer::ExperienceWriter;
use crate::learning::feedback::{
    excursion, qualify, ExitReason, QualificationPolicy, TradeOutcome, PNL_SOURCE_MODEL,
};
use crate::learning::outcome_store::OutcomeStore;
use crate::okx::client::OKXClient;
use crate::okx::trading::AuditEntry;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Everything about a decision that an outcome needs to inherit.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SignalContext {
    #[serde(default)]
    pub decision_record_id: String,
    #[serde(default)]
    pub strategy_id: String,
    #[serde(default)]
    pub strategy_version: String,
    #[serde(default)]
    pub prompt_version: String,
    #[serde(default)]
    pub prompt_hash: String,
    #[serde(default)]
    pub cycle_position: String,
    #[serde(default)]
    pub detected_patterns: Vec<String>,
}

impl SignalContext {
    /// The audit row already carries the receipt linkage and market context,
    /// so reconciliation needs no secondary lookup.
    pub fn from_audit(entry: &AuditEntry) -> Self {
        Self {
            decision_record_id: entry.decision_record_id.clone(),
            strategy_id: entry.strategy_id.clone(),
            strategy_version: entry.strategy_version.clone(),
            prompt_version: entry.prompt_version.clone(),
            prompt_hash: entry.prompt_hash.clone(),
            cycle_position: entry.cycle_position.clone(),
            detected_patterns: entry.detected_patterns.clone(),
        }
    }
}

/// Result of walking forward from an entry, in R.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resolved {
    pub exit_price: f64,
    pub exit_reason: ExitReason,
    pub hold_bars: u32,
    pub mfe_r: f64,
    pub mae_r: f64,
}

/// Walk bars forward from an entry until stop, target or the holding limit is
/// reached. Returns `None` while the trade is still open, so an unresolved
/// trade is never written as a result.
///
/// Bars must be oldest-first and start at (or after) the entry. Stop is
/// evaluated before target within the same bar because intra-bar ordering is
/// unknown and this project always takes the pessimistic branch.
pub fn resolve_exit(
    side: &str,
    entry: f64,
    stop: f64,
    target: f64,
    bars: &[crate::learning::feedback::BarHlc],
    max_hold_bars: u32,
) -> Option<Resolved> {
    let sign = crate::learning::feedback::side_sign(side)?;
    let risk = (entry - stop).abs();
    if risk <= 0.0 || !entry.is_finite() || !stop.is_finite() {
        return None;
    }

    let mut mfe = 0.0_f64;
    let mut mae = 0.0_f64;

    for (idx, bar) in bars.iter().enumerate() {
        if !bar.high.is_finite() || !bar.low.is_finite() {
            continue;
        }
        let best = if sign > 0.0 { bar.high } else { bar.low };
        let worst = if sign > 0.0 { bar.low } else { bar.high };
        mfe = mfe.max((best - entry) * sign / risk);
        mae = mae.max((entry - worst) * sign / risk);

        let hold_bars = (idx + 1) as u32;
        let stop_hit = if sign > 0.0 { bar.low <= stop } else { bar.high >= stop };
        let target_hit = if sign > 0.0 { bar.high >= target } else { bar.low <= target };

        if stop_hit {
            return Some(Resolved {
                exit_price: stop,
                exit_reason: ExitReason::StopLoss,
                hold_bars,
                mfe_r: mfe.max(0.0),
                mae_r: mae.max(0.0),
            });
        }
        if target_hit {
            return Some(Resolved {
                exit_price: target,
                exit_reason: ExitReason::TakeProfit,
                hold_bars,
                mfe_r: mfe.max(0.0),
                mae_r: mae.max(0.0),
            });
        }
        if max_hold_bars > 0 && hold_bars >= max_hold_bars {
            return Some(Resolved {
                exit_price: bar.close,
                exit_reason: ExitReason::Expired,
                hold_bars,
                mfe_r: mfe.max(0.0),
                mae_r: mae.max(0.0),
            });
        }
    }

    // Ran out of bars without resolution: still open.
    None
}

/// Inputs describing one resolved execution.
pub struct ResolutionInput<'a> {
    pub signal_id: &'a str,
    pub context: &'a SignalContext,
    pub symbol: &'a str,
    pub timeframe: &'a str,
    pub side: &'a str,
    pub entry: f64,
    pub stop: f64,
    pub target: f64,
    pub size: f64,
    pub filled: bool,
    pub fill_ratio: f64,
    pub created_ms: i64,
    pub bars: &'a [crate::learning::feedback::BarHlc],
    pub max_hold_bars: u32,
    pub policy: &'a QualificationPolicy,
    pub hit_exit_reason: Option<ExitReason>,
}

/// Assemble a `TradeOutcome` from an execution and the market that followed.
///
/// Returns `None` only when a filled trade is still open and therefore has no
/// result yet. Unfilled orders do produce an outcome, so the reason a signal
/// never became a position stays auditable.
pub fn build_outcome(input: &ResolutionInput<'_>) -> Option<TradeOutcome> {
    let now = crate::util::timefmt::now_local_ms();
    let risk = (input.entry - input.stop).abs();

    if !input.filled {
        let mut outcome = base_outcome(input, now);
        outcome.exit_reason = ExitReason::Unfilled.as_str().to_string();
        outcome.r_multiple = 0.0;
        outcome.pnl_source = PNL_SOURCE_MODEL.to_string();
        let (qualified, reason) = qualify(&outcome, input.policy);
        outcome.qualified = qualified;
        outcome.qualification_reason = reason;
        return Some(outcome);
    }

    let resolved = resolve_exit(
        input.side,
        input.entry,
        input.stop,
        input.target,
        input.bars,
        input.max_hold_bars,
    )?;

    // A management action (early close / moved stop) reported by the caller
    // overrides the mechanical walk-forward result.
    let exit_reason = input.hit_exit_reason.unwrap_or(resolved.exit_reason);
    let sign = crate::learning::feedback::side_sign(input.side).unwrap_or(1.0);
    let r_multiple = if risk > 0.0 { (resolved.exit_price - input.entry) * sign / risk } else { 0.0 };
    let notional = (input.entry + resolved.exit_price).abs() * input.size;
    let rate = crate::strategies::cost_rate(input.symbol);

    let mut outcome = base_outcome(input, now);
    outcome.exit_price = Some(resolved.exit_price);
    outcome.exit_reason = exit_reason.as_str().to_string();
    outcome.mfe_r = resolved.mfe_r;
    outcome.mae_r = resolved.mae_r;
    outcome.hold_bars = resolved.hold_bars;
    outcome.r_multiple = r_multiple;
    outcome.fees_usd = notional * rate;
    outcome.realized_pnl_usd = (resolved.exit_price - input.entry) * sign * input.size;
    // Derived from price and size, never from venue fills. Labelled so the
    // console and any later analysis cannot mistake it for reconciled PnL.
    outcome.pnl_source = PNL_SOURCE_MODEL.to_string();
    outcome.resolved_ms = now;

    let (qualified, reason) = qualify(&outcome, input.policy);
    outcome.qualified = qualified;
    outcome.qualification_reason = reason;
    Some(outcome)
}

fn base_outcome(input: &ResolutionInput<'_>, now: i64) -> TradeOutcome {
    TradeOutcome {
        signal_id: input.signal_id.to_string(),
        decision_record_id: input.context.decision_record_id.clone(),
        strategy_id: input.context.strategy_id.clone(),
        strategy_version: input.context.strategy_version.clone(),
        prompt_version: input.context.prompt_version.clone(),
        prompt_hash: input.context.prompt_hash.clone(),
        symbol: input.symbol.to_string(),
        timeframe: input.timeframe.to_string(),
        side: crate::learning::feedback::normalize_side(input.side)
            .unwrap_or("unknown")
            .to_string(),
        cycle_position: input.context.cycle_position.clone(),
        detected_patterns: input.context.detected_patterns.clone(),
        entry_price: input.entry,
        stop_price: input.stop,
        target_price: input.target,
        exit_price: None,
        size: input.size,
        filled: input.filled,
        fill_ratio: input.fill_ratio,
        fees_usd: 0.0,
        realized_pnl_usd: 0.0,
        pnl_source: String::new(),
        r_multiple: 0.0,
        mfe_r: 0.0,
        mae_r: 0.0,
        hold_bars: 0,
        exit_reason: String::new(),
        qualified: false,
        qualification_reason: String::new(),
        created_ms: input.created_ms,
        resolved_ms: now,
    }
}

/// Summary of one reconciliation pass, surfaced in logs and the console.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReconcileReport {
    pub checked: usize,
    pub resolved: usize,
    pub still_open: usize,
    pub unfilled: usize,
    pub experiences_written: usize,
    pub skipped_existing: usize,
    pub errors: Vec<String>,
}

impl ReconcileReport {
    pub fn is_empty(&self) -> bool {
        self.checked == 0
    }
}

/// Reconciles audit rows against the venue.
pub struct Reconciler {
    client: OKXClient,
    store: OutcomeStore,
    experience: ExperienceWriter,
}

impl Reconciler {
    pub fn new(client: OKXClient, store: OutcomeStore, experience: ExperienceWriter) -> Self {
        Self { client, store, experience }
    }

    pub fn store(&self) -> &OutcomeStore {
        &self.store
    }

    pub fn experience(&self) -> &ExperienceWriter {
        &self.experience
    }

    /// Resolve every submitted audit row that has no outcome yet.
    ///
    /// `max_age_ms` bounds how far back a single pass reaches, so a first run
    /// against a long audit log does not hammer the venue.
    pub async fn reconcile(
        &self,
        entries: &[AuditEntry],
        policy: &QualificationPolicy,
        max_hold_bars: u32,
        max_age_ms: i64,
        write_experience: bool,
    ) -> ReconcileReport {
        let mut report = ReconcileReport::default();
        let now = crate::util::timefmt::now_local_ms();

        for entry in entries {
            if !entry.submitted || entry.signal_id.is_empty() {
                continue;
            }
            if self.store.exists(&entry.signal_id) {
                report.skipped_existing += 1;
                continue;
            }
            if max_age_ms > 0 && now.saturating_sub(entry.timestamp_ms) > max_age_ms {
                continue;
            }
            report.checked += 1;

            let context = SignalContext::from_audit(entry);
            match self.resolve_one(entry, &context, policy, max_hold_bars).await {
                Ok(Some(outcome)) => {
                    if outcome.filled {
                        report.resolved += 1;
                    } else {
                        report.unfilled += 1;
                    }
                    if let Err(e) = self.store.save(&outcome) {
                        report.errors.push(format!("{} 保存结果失败: {e:#}", entry.signal_id));
                        continue;
                    }
                    if write_experience {
                        match self.experience.record(&outcome) {
                            Ok(Some(_)) => report.experiences_written += 1,
                            Ok(None) => {}
                            Err(e) => report.errors.push(format!("{} 写入经验库失败: {e:#}", entry.signal_id)),
                        }
                    }
                }
                Ok(None) => report.still_open += 1,
                Err(e) => {
                    report.errors.push(format!("{} 对账失败: {e:#}", entry.signal_id));
                }
            }
        }

        if report.checked > 0 {
            info!(
                "对账完成: 检查 {} / 已解决 {} / 仍持有 {} / 未成交 {} / 写入经验 {}",
                report.checked, report.resolved, report.still_open, report.unfilled, report.experiences_written
            );
        }
        report
    }

    async fn resolve_one(
        &self,
        entry: &AuditEntry,
        context: &SignalContext,
        policy: &QualificationPolicy,
        max_hold_bars: u32,
    ) -> anyhow::Result<Option<TradeOutcome>> {
        // A failed lookup must not be silently treated as "unfilled".
        let order = self
            .client
            .get_order(&entry.instrument, Some(&entry.order_id), None)
            .await
            .map_err(|e| anyhow::anyhow!("查询订单状态失败: {e:#}"))?;

        let state_str = order.get("state").and_then(|s| s.as_str()).unwrap_or("").to_string();
        let acc = order
            .get("accFillSz")
            .and_then(|s| s.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let sz = order
            .get("sz")
            .and_then(|s| s.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let avg_px = order
            .get("avgPx")
            .and_then(|s| s.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|p| *p > 0.0);
        let filled = state_str == "filled" || (state_str == "partially_filled" && acc > 0.0);
        let fill_ratio = if sz > 0.0 {
            (acc / sz).clamp(0.0, 1.0)
        } else if filled {
            1.0
        } else {
            0.0
        };

        // Unfilled entries are only conclusive once the order can no longer
        // fill: a still-live order may yet become a position.
        if !filled && (state_str == "live" || state_str == "partially_filled") {
            return Ok(None);
        }

        let (entry_px, stop_px, target_px) = price_levels(entry, avg_px);
        if entry_px <= 0.0 || stop_px <= 0.0 || target_px <= 0.0 {
            anyhow::bail!("审计记录缺少可用的入场/止损/止盈价");
        }

        let bars = if filled {
            self.fetch_bars_after(&entry.instrument, &entry.timeframe, entry.timestamp_ms)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let input = ResolutionInput {
            signal_id: &entry.signal_id,
            context,
            symbol: &entry.instrument,
            timeframe: &entry.timeframe,
            side: &entry.direction,
            entry: entry_px,
            stop: stop_px,
            target: target_px,
            size: entry.size.as_ref().and_then(|v| v.as_f64()).unwrap_or(0.0),
            filled,
            fill_ratio,
            created_ms: entry.timestamp_ms,
            bars: &bars,
            max_hold_bars,
            policy,
            hit_exit_reason: None,
        };
        Ok(build_outcome(&input))
    }

    /// Closed bars at or after `since_ms`, oldest first.
    async fn fetch_bars_after(
        &self,
        inst_id: &str,
        timeframe: &str,
        since_ms: i64,
    ) -> anyhow::Result<Vec<crate::learning::feedback::BarHlc>> {
        let rows = self
            .client
            .get_candles_paginated(inst_id, timeframe, 300, true)
            .await?;
        let mut bars: Vec<crate::learning::feedback::BarHlc> = rows
            .iter()
            .filter(|r| r.len() >= 5)
            .filter_map(|r| {
                let ts = r[0].parse::<i64>().ok()?;
                if ts < since_ms {
                    return None;
                }
                let high = r[2].parse::<f64>().ok()?;
                let low = r[3].parse::<f64>().ok()?;
                let close = r[4].parse::<f64>().ok()?;
                Some((ts, crate::learning::feedback::BarHlc::new(high, low, close)))
            })
            .map(|(_, b)| b)
            .collect();
        bars.reverse(); // OKX returns newest first
        debug!("对账载入 {} 根 {} K 线", bars.len(), timeframe);
        Ok(bars)
    }
}

fn price_levels(entry: &AuditEntry, avg_px: Option<f64>) -> (f64, f64, f64) {
    let num = |v: &Option<serde_json::Value>| v.as_ref().and_then(|x| x.as_f64()).unwrap_or(0.0);
    let entry_px = avg_px.unwrap_or_else(|| num(&entry.price));
    (entry_px, num(&entry.stop_loss_price), num(&entry.take_profit_price))
}

/// Best-effort exit classification for a management action, so an early close
/// is not misreported as a stop-out.
pub fn classify_management(action: &str) -> Option<ExitReason> {
    match action.trim().to_uppercase().as_str() {
        "CLOSE_EARLY" => Some(ExitReason::ClosedEarly),
        _ => None,
    }
}

/// Helper for callers that only have bars and want MFE/MAE for display.
pub fn excursions_for(
    side: &str,
    entry: f64,
    stop: f64,
    bars: &[crate::learning::feedback::BarHlc],
) -> Option<crate::learning::feedback::Excursion> {
    excursion(side, entry, stop, None, bars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::feedback::BarHlc;

    fn input_bars() -> Vec<BarHlc> {
        vec![
            BarHlc::new(100.6, 99.4, 100.2),
            BarHlc::new(101.2, 99.8, 101.0),
            BarHlc::new(102.5, 100.5, 102.0),
        ]
    }

    #[test]
    fn resolve_exit_prefers_stop_when_both_sides_touched() {
        // Bar spans both stop and target: must report the stop, the
        // pessimistic assumption.
        let bars = vec![BarHlc::new(105.0, 98.0, 100.0)];
        let r = resolve_exit("long", 100.0, 99.0, 104.0, &bars, 10).unwrap();
        assert_eq!(r.exit_reason, ExitReason::StopLoss);
        assert_eq!(r.exit_price, 99.0);
    }

    #[test]
    fn resolve_exit_returns_none_while_open() {
        // Only one bar, no level touched, holding limit not reached.
        let bars = vec![BarHlc::new(100.5, 99.5, 100.2)];
        assert!(resolve_exit("long", 100.0, 99.0, 104.0, &bars, 10).is_none());
    }

    #[test]
    fn resolve_exit_expires_at_holding_limit() {
        let bars = input_bars();
        let r = resolve_exit("long", 100.0, 99.0, 110.0, &bars, 2).unwrap();
        assert_eq!(r.exit_reason, ExitReason::Expired);
        assert_eq!(r.hold_bars, 2);
        assert_eq!(r.exit_price, 101.0);
    }

    #[test]
    fn build_outcome_records_target_hit_as_positive_r() {
        let policy = QualificationPolicy { require_filled: true, min_hold_bars: 1, max_abs_r: 25.0 };
        let ctx = SignalContext {
            decision_record_id: "rec".into(),
            strategy_id: "2pa_trend".into(),
            strategy_version: "2026-09-v1".into(),
            prompt_version: "v1".into(),
            prompt_hash: "h".into(),
            cycle_position: "trending_tr".into(),
            detected_patterns: vec!["h2".into()],
        };
        let bars = vec![BarHlc::new(102.5, 99.5, 102.0)];
        let input = ResolutionInput {
            signal_id: "sig1",
            context: &ctx,
            symbol: "BTC-USDT-SWAP",
            timeframe: "15m",
            side: "做多", // Chinese input must normalise to long
            entry: 100.0,
            stop: 99.0,
            target: 102.0,
            size: 1.0,
            filled: true,
            fill_ratio: 1.0,
            created_ms: 1_700_000_000_000,
            bars: &bars,
            max_hold_bars: 10,
            policy: &policy,
            hit_exit_reason: None,
        };
        let o = build_outcome(&input).unwrap();
        assert_eq!(o.side, "long");
        assert_eq!(o.exit_reason, "take_profit");
        assert_eq!(o.r_multiple, 2.0);
        assert_eq!(o.mfe_r, 2.5);
        assert!(o.qualified, "{}", o.qualification_reason);
        assert_eq!(o.pnl_source, PNL_SOURCE_MODEL);
        assert_eq!(o.strategy_id, "2pa_trend");
        assert_eq!(o.decision_record_id, "rec");
    }

    #[test]
    fn build_outcome_marks_unfilled_as_unqualified() {
        let policy = QualificationPolicy::default();
        let ctx = SignalContext::default();
        let input = ResolutionInput {
            signal_id: "sig2",
            context: &ctx,
            symbol: "BTC-USDT-SWAP",
            timeframe: "15m",
            side: "short",
            entry: 100.0,
            stop: 101.0,
            target: 98.0,
            size: 1.0,
            filled: false,
            fill_ratio: 0.0,
            created_ms: 1,
            bars: &[],
            max_hold_bars: 10,
            policy: &policy,
            hit_exit_reason: None,
        };
        let o = build_outcome(&input).unwrap();
        assert!(!o.filled);
        assert!(!o.qualified);
        assert_eq!(o.exit_reason, "unfilled");
    }

    #[test]
    fn build_outcome_returns_none_for_open_trade() {
        let policy = QualificationPolicy::default();
        let ctx = SignalContext::default();
        let bars = vec![BarHlc::new(100.4, 99.6, 100.2)];
        let input = ResolutionInput {
            signal_id: "sig3",
            context: &ctx,
            symbol: "BTC-USDT-SWAP",
            timeframe: "15m",
            side: "long",
            entry: 100.0,
            stop: 99.0,
            target: 104.0,
            size: 1.0,
            filled: true,
            fill_ratio: 1.0,
            created_ms: 1,
            bars: &bars,
            max_hold_bars: 50,
            policy: &policy,
            hit_exit_reason: None,
        };
        assert!(build_outcome(&input).is_none());
    }

    #[test]
    fn management_close_overrides_mechanical_exit() {
        assert_eq!(classify_management("CLOSE_EARLY"), Some(ExitReason::ClosedEarly));
        assert_eq!(classify_management("HOLD"), None);
    }
}
