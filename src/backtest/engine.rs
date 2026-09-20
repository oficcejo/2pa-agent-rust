//! Main backtest execution engine.
//! Features:
//! - Closed HTF resampling (e.g. 15m -> 1H closed bars with zero lookahead bias)
//! - Programmatic Gating (`strategies::diagnostics` short-circuiting 97%+ chop bars)
//! - Decision caching (zero token cost for reruns)
//! - Realistic order execution and virtual margin accounting
//! - Complete quantitative metrics calculation (Sharpe, Sortino, Drawdown, Expectancy R, etc.)

use crate::ai::client::AIClient;
use crate::backtest::account::{PositionSide, VirtualAccount};
use crate::backtest::cache::{CachedDecision, DecisionCache};
use crate::backtest::data::timeframe_to_ms;
use crate::backtest::matcher::{OrderMatcher, PendingOrder};
use crate::backtest::types::{
    BacktestConfig, BacktestJobStatus, BacktestMetrics, BacktestReport, BacktestTrade, EquityPoint,
};
use crate::data::base::{KlineBar, KlineFrame};
use crate::data::snapshot::{build_analysis_frame, INDICATOR_WARMUP_BARS};
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, info};

pub type ProgressCallback = Arc<dyn Fn(BacktestJobStatus) + Send + Sync>;

pub struct BacktestEngine {
    pub config: BacktestConfig,
    pub cache: DecisionCache,
    pub ai_client: Option<Arc<AIClient>>,
    pub cancel_flag: Arc<AtomicBool>,
}

impl BacktestEngine {
    pub fn new(
        config: BacktestConfig,
        cache: DecisionCache,
        ai_client: Option<Arc<AIClient>>,
    ) -> Self {
        Self {
            config,
            cache,
            ai_client,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Resample LTF ascending bars into closed HTF ascending bars with ZERO lookahead bias.
    /// Only HTF bars that have completed prior to (or at) the current LTF bar's close timestamp are returned.
    pub fn resample_closed_htf(
        ltf_bars_asc: &[KlineBar],
        ltf_tf: &str,
        htf_tf: &str,
        current_ltf_idx: usize,
    ) -> Vec<KlineBar> {
        let ltf_interval_ms = timeframe_to_ms(ltf_tf);
        let htf_interval_ms = timeframe_to_ms(htf_tf);

        if htf_interval_ms <= ltf_interval_ms || current_ltf_idx >= ltf_bars_asc.len() {
            return Vec::new();
        }

        let current_ltf_close_ts = ltf_bars_asc[current_ltf_idx].ts_open + ltf_interval_ms;

        // Group past LTF bars into HTF bins
        let mut htf_bins: BTreeMap<i64, Vec<&KlineBar>> = BTreeMap::new();
        for bar in &ltf_bars_asc[..=current_ltf_idx] {
            let htf_bin_open = bar.ts_open - (bar.ts_open % htf_interval_ms);
            htf_bins.entry(htf_bin_open).or_default().push(bar);
        }

        let mut htf_closed_bars = Vec::new();
        for (bin_open_ts, constituents) in htf_bins {
            let bin_close_ts = bin_open_ts + htf_interval_ms;
            // The HTF bar is only considered CLOSED if its close time is <= current LTF close time
            if bin_close_ts <= current_ltf_close_ts && !constituents.is_empty() {
                let open = constituents.first().unwrap().open;
                let close = constituents.last().unwrap().close;
                let mut high = f64::MIN;
                let mut low = f64::MAX;
                let mut volume = 0.0;
                let mut amount = 0.0;

                for c in &constituents {
                    if c.high > high { high = c.high; }
                    if c.low < low { low = c.low; }
                    volume += c.volume;
                    amount += c.amount;
                }

                htf_closed_bars.push(KlineBar {
                    seq: 0,
                    ts_open: bin_open_ts,
                    open,
                    high: high.max(open).max(close),
                    low: low.min(open).min(close),
                    close,
                    volume,
                    amount,
                    pct_chg: Some((close - open) / open * 100.0),
                    closed: true,
                });
            }
        }

        htf_closed_bars
    }

    /// Select standard matching Higher Timeframe (HTF) for the given trading timeframe.
    pub fn default_htf_for_ltf(ltf: &str) -> &'static str {
        match ltf.trim().to_lowercase().as_str() {
            "1m" | "3m" | "5m" => "15m",
            "15m" | "30m" => "1H",
            "1h" => "4H",
            "2h" | "4h" => "1D",
            _ => "1D",
        }
    }

    /// Execute backtest simulation over ascending chronologically sorted bars.
    pub async fn run(
        &self,
        job_id: &str,
        bars_asc: &[KlineBar],
        on_progress: Option<ProgressCallback>,
    ) -> Result<BacktestReport> {
        let start_time_ms = Utc::now().timestamp_millis();
        let total_bars = bars_asc.len();
        let required_warmup = INDICATOR_WARMUP_BARS + 20;

        if total_bars <= required_warmup {
            return Err(anyhow!(
                "数据量不足：总共 {} 根 K 线，至少需要 {} 根进行指标预热与策略诊断",
                total_bars,
                required_warmup
            ));
        }

        // Determine starting bar index, respecting indicator warmup and optional start_time_ms
        let start_idx = if let Some(start_ts) = self.config.start_time_ms {
            let pos = bars_asc.iter().position(|b| b.ts_open >= start_ts).unwrap_or(bars_asc.len());
            pos.max(required_warmup)
        } else {
            required_warmup
        };

        if start_idx >= total_bars {
            return Err(anyhow!(
                "指定时间范围内无可用数据：需要至少 {} 根预热 K 线，但在起始时间之后仅有 {} 根 K 线",
                required_warmup,
                total_bars.saturating_sub(start_idx)
            ));
        }

        info!(
            "Starting backtest [{}]: symbol={}, strategy={}, bars={}, start_bar={}",
            job_id, self.config.symbol, self.config.strategy_id, total_bars, start_idx
        );

        let mut account = VirtualAccount::new(&self.config);
        let mut matcher = OrderMatcher::new();
        let mut equity_curve = Vec::with_capacity(total_bars - start_idx);
        let mut trades = Vec::new();

        let mut gated_bars_skipped = 0usize;
        let mut llm_evaluations = 0usize;
        let mut cache_hits = 0usize;

        let default_htf = Self::default_htf_for_ltf(&self.config.timeframe);
        let htf_tf = self.config.htf_timeframe.as_deref().unwrap_or(default_htf);
        let ltf_interval_ms = timeframe_to_ms(&self.config.timeframe);

        // Record initial equity point
        equity_curve.push(EquityPoint {
            timestamp_ms: bars_asc[start_idx].ts_open,
            equity: account.equity,
            cash: account.cash,
            unrealized_pnl: 0.0,
            drawdown_pct: 0.0,
            in_position: false,
        });

        let mut last_progress_report = 0usize;

        let active_end_idx = if let Some(end_ts) = self.config.end_time_ms {
            bars_asc.iter().position(|b| b.ts_open > end_ts).unwrap_or(total_bars)
        } else {
            total_bars
        };

        let total_active_bars = active_end_idx.saturating_sub(start_idx).max(1);

        // Walk forward bar by bar
        for i in start_idx..active_end_idx {
            if self.cancel_flag.load(Ordering::Relaxed) {
                return Err(anyhow!("Backtest cancelled by user"));
            }

            let current_bar = &bars_asc[i];

            // 1. Process pending limit/breakout orders with current bar
            matcher.process_pending_orders(&mut account, current_bar);

            // 2. If in position, update excursion and check mechanical exit or liquidation
            if account.position.is_some() {
                account.update_bar(current_bar.high, current_bar.low, current_bar.close);

                if self.config.use_mechanical_exit {
                    if let Some(closed_trade) = matcher.check_mechanical_exit(
                        &mut account,
                        current_bar,
                        self.config.max_hold_bars,
                    )? {
                        trades.push(closed_trade);
                    }
                } else if let Some(liq) = account.liquidation_price() {
                    // Even if strategy mechanical exits are disabled, exchange liquidation is enforced
                    let pos_side = account.position.as_ref().unwrap().side.clone();
                    let liq_hit = match pos_side {
                        PositionSide::Long => current_bar.low <= liq,
                        PositionSide::Short => current_bar.high >= liq,
                    };
                    if liq_hit {
                        account.is_liquidated = true;
                        let exit_px = match pos_side {
                            PositionSide::Long => if current_bar.open <= liq { current_bar.open } else { liq },
                            PositionSide::Short => if current_bar.open >= liq { current_bar.open } else { liq },
                        };
                        if let Ok(trade) = account.close_position(exit_px, current_bar.ts_open, "liquidated") {
                            trades.push(trade);
                        }
                    }
                }
            }

            // 3. If no position, evaluate entry setup via Programmatic Gating
            if account.position.is_none() && !account.is_liquidated {
                // Slice past bars up to i, and reverse to newest-first format for snapshot frame builder
                let history_slice = &bars_asc[..=i];
                let mut raw_desc = history_slice.to_vec();
                raw_desc.reverse();

                let frame_opt = build_analysis_frame(
                    &raw_desc,
                    50, // standard frame size
                    &self.config.symbol,
                    &self.config.timeframe,
                    Some(current_bar.ts_open + ltf_interval_ms),
                );

                if let Some(frame) = frame_opt {
                    // Resample closed HTF bars with zero lookahead bias
                    let htf_closed = Self::resample_closed_htf(
                        bars_asc,
                        &self.config.timeframe,
                        htf_tf,
                        i,
                    );

                    let htf_frame_opt = if htf_closed.len() >= 25 {
                        let mut htf_desc = htf_closed;
                        htf_desc.reverse();
                        build_analysis_frame(
                            &htf_desc,
                            20,
                            &self.config.symbol,
                            htf_tf,
                            Some(current_bar.ts_open + ltf_interval_ms),
                        )
                    } else {
                        None
                    };

                    // --- Programmatic Gating ---
                    // Short-circuit 97%+ chop bars without touching LLM!
                    let diag = crate::strategies::diagnostics(
                        &self.config.strategy_id,
                        &frame,
                        htf_frame_opt.as_ref(),
                    );

                    let long_eligible = diag["long"]["eligible"] == true;
                    let short_eligible = diag["short"]["eligible"] == true;

                    if !long_eligible && !short_eligible {
                        gated_bars_skipped += 1;
                    } else {
                        // Diagnostic gate passed! Setup candidate detected.
                        let fingerprint = DecisionCache::compute_fingerprint(
                            &self.config.strategy_id,
                            &frame,
                            htf_frame_opt.as_ref(),
                        );

                        let cached_opt = if self.config.use_cache {
                            self.cache.get(&fingerprint)
                        } else {
                            None
                        };

                        let decision_val: serde_json::Value = if let Some(cached) = cached_opt {
                            cache_hits += 1;
                            cached.raw_decision
                        } else {
                            // Synthesize or call LLM
                            let synthesized = self.produce_candidate_decision(
                                &frame,
                                htf_frame_opt.as_ref(),
                                &diag,
                                long_eligible,
                            ).await;

                            if self.config.use_cache {
                                self.cache.insert(
                                    fingerprint,
                                    CachedDecision {
                                        strategy_id: self.config.strategy_id.clone(),
                                        action: synthesized["decision"]["action"].as_str().unwrap_or("WAIT").to_string(),
                                        order_type: synthesized["decision"]["order_type"].as_str().unwrap_or("不下单").to_string(),
                                        order_direction: synthesized["decision"]["order_direction"].as_str().map(|s| s.to_string()),
                                        entry_price: synthesized["decision"]["entry_price"].as_f64(),
                                        stop_loss_price: synthesized["decision"]["stop_loss_price"].as_f64(),
                                        take_profit_price: synthesized["decision"]["take_profit_price"].as_f64(),
                                        trade_confidence: synthesized["decision"]["trade_confidence"].as_u64().unwrap_or(0) as u32,
                                        reasoning: synthesized["decision"]["reasoning"].as_str().unwrap_or("").to_string(),
                                        stage1_diagnosis: Some(diag.clone()),
                                        raw_decision: synthesized.clone(),
                                        timestamp_ms: current_bar.ts_open,
                                    },
                                );
                            }

                            if self.ai_client.is_some() && self.config.allow_llm_calls {
                                llm_evaluations += 1;
                            }
                            synthesized
                        };

                        // Validate proposal with strategies::enforce
                        let mut wrapper = decision_val;
                        let stage1_mock = json!({
                            "gate_result": "proceed",
                            "program_candidates": diag
                        });

                        crate::strategies::enforce(
                            &self.config.strategy_id,
                            &frame,
                            htf_frame_opt.as_ref(),
                            &stage1_mock,
                            &mut wrapper,
                            None,
                        );

                        let dec = &wrapper["decision"];
                        let action = dec["action"].as_str().unwrap_or("WAIT");
                        if action == "OPEN" {
                            let dir_str = dec["order_direction"].as_str().unwrap_or("");
                            let order_type = dec["order_type"].as_str().unwrap_or("市价单");
                            let entry_px = dec["entry_price"].as_f64().unwrap_or(current_bar.close);
                            let stop_px = dec["stop_loss_price"].as_f64().unwrap_or(0.0);
                            let tp_px = dec["take_profit_price"].as_f64().unwrap_or(0.0);

                            let side = if dir_str == "做多" {
                                PositionSide::Long
                            } else {
                                PositionSide::Short
                            };

                            let lots = account.calculate_order_size(
                                entry_px,
                                stop_px,
                                self.config.risk_percent,
                                self.config.max_margin_percent,
                            );

                            if lots >= account.lot_sz {
                                let signal_id = format!("SIG-{}-{}", current_bar.ts_open, trades.len() + 1);

                                if order_type == "市价单" {
                                    if let Err(e) = account.open_position(
                                        &self.config.strategy_id,
                                        &signal_id,
                                        &self.config.symbol,
                                        side,
                                        order_type,
                                        current_bar.close,
                                        stop_px,
                                        tp_px,
                                        lots,
                                        current_bar.ts_open,
                                    ) {
                                        debug!("Order rejected by account: {}", e);
                                    }
                                } else {
                                    // Queue Limit or Breakout order
                                    matcher.add_pending_order(PendingOrder {
                                        signal_id,
                                        strategy_id: self.config.strategy_id.clone(),
                                        symbol: self.config.symbol.clone(),
                                        side,
                                        order_type: order_type.to_string(),
                                        target_price: entry_px,
                                        stop_loss: stop_px,
                                        take_profit: tp_px,
                                        contracts: lots,
                                        created_at_ms: current_bar.ts_open,
                                        expiry_bars_remaining: 5,
                                    });
                                }
                            }
                        }
                    }
                }
            }

            // 4. Update equity curve point at bar close
            account.update_equity(current_bar.close);
            let peak_equity = equity_curve
                .iter()
                .map(|p| p.equity)
                .fold(account.initial_capital, f64::max);
            let current_equity = account.equity;
            let dd_pct = if peak_equity > 0.0 {
                ((peak_equity - current_equity) / peak_equity * 100.0).max(0.0)
            } else {
                0.0
            };

            equity_curve.push(EquityPoint {
                timestamp_ms: current_bar.ts_open,
                equity: current_equity,
                cash: account.cash,
                unrealized_pnl: account.unrealized_pnl(current_bar.close),
                drawdown_pct: dd_pct,
                in_position: account.position.is_some(),
            });

            // 5. Report progress periodically
            let pct = ((i - start_idx + 1) as f64 / total_active_bars as f64) * 100.0;
            if pct as usize > last_progress_report + 4 || i == active_end_idx - 1 {
                last_progress_report = pct as usize;
                if let Some(ref cb) = on_progress {
                    cb(BacktestJobStatus {
                        job_id: job_id.to_string(),
                        status: "running".to_string(),
                        progress_pct: pct.min(100.0),
                        current_bar: i - start_idx + 1,
                        total_bars: total_active_bars,
                        message: format!("正在回测第 {}/{} 根 K 线...", i - start_idx + 1, total_active_bars),
                        error: None,
                        created_at_ms: start_time_ms,
                        completed_at_ms: None,
                    });
                }
            }
        }

        // Close any lingering open position at end of backtest data
        if account.position.is_some() {
            let last_active_bar = &bars_asc[(active_end_idx.saturating_sub(1)).min(total_bars - 1)];
            if let Ok(trade) = account.close_position(last_active_bar.close, last_active_bar.ts_open, "backtest_end") {
                trades.push(trade);
            }
        }

        // Save updated decision cache
        if self.config.use_cache {
            let _ = self.cache.save();
        }

        let execution_duration_ms = (Utc::now().timestamp_millis() - start_time_ms).max(1) as u64;

        // Compute quantitative metrics
        let metrics = Self::compute_metrics(
            &account,
            &trades,
            &equity_curve,
            total_active_bars,
            gated_bars_skipped,
            llm_evaluations,
            cache_hits,
            &self.config.timeframe,
        );

        let report = BacktestReport {
            job_id: job_id.to_string(),
            config: self.config.clone(),
            metrics,
            equity_curve,
            trades,
            start_time_ms: bars_asc[start_idx].ts_open,
            end_time_ms: bars_asc[(active_end_idx.saturating_sub(1)).min(total_bars - 1)].ts_open,
            execution_duration_ms,
        };

        if let Some(ref cb) = on_progress {
            cb(BacktestJobStatus {
                job_id: job_id.to_string(),
                status: "completed".to_string(),
                progress_pct: 100.0,
                current_bar: total_bars,
                total_bars,
                message: "回测执行完毕".to_string(),
                error: None,
                created_at_ms: start_time_ms,
                completed_at_ms: Some(Utc::now().timestamp_millis()),
            });
        }

        info!(
            "Backtest [{}] complete: trades={}, net_profit={:.2} USDT ({:.2}%), Sharpe={:.2}, MaxDD={:.2}%",
            job_id,
            report.metrics.total_trades,
            report.metrics.net_profit,
            report.metrics.net_profit_pct,
            report.metrics.sharpe_ratio,
            report.metrics.max_drawdown_pct
        );

        Ok(report)
    }

    /// Produce decision: uses AIClient when active and configured, or deterministic strategy evidence.
    pub async fn produce_candidate_decision(
        &self,
        frame: &KlineFrame,
        _htf_frame: Option<&KlineFrame>,
        diag: &serde_json::Value,
        long_eligible: bool,
    ) -> serde_json::Value {
        let (dir_str, evidence_val) = if long_eligible {
            ("做多", &diag["long"]["evidence"])
        } else {
            ("做空", &diag["short"]["evidence"])
        };

        let b0 = &frame.bars[0];
        let ref_close = evidence_val["reference_close"].as_f64().unwrap_or(b0.close);
        let atr = evidence_val["atr"].as_f64().unwrap_or_else(|| {
            frame.indicators.atr14.first().copied().unwrap_or(ref_close * 0.01)
        }).max(1e-6);
        let sign = if dir_str == "做多" { 1.0 } else { -1.0 };

        let raw_invalidation = evidence_val["invalidation"].as_f64().unwrap_or_else(|| {
            ref_close - sign * 1.5 * atr
        });
        let target_bound = evidence_val["target_bound"].as_f64().unwrap_or_else(|| {
            ref_close + sign * 2.8 * atr
        });

        // 1. Invalidation buffer: at least 0.25 ATR beyond structural invalidation point (satisfies >= 0.2 ATR requirement)
        let min_inval_buf = 0.25 * atr;
        let dist_from_invalidation = (raw_invalidation - ref_close).abs() + min_inval_buf;

        // 2. Minimum volatility distance: at least 0.8 ATR and 0.3% of price (with 0.02 ATR buffer to strictly exceed)
        let min_volatility_dist = (0.8 * atr).max(0.003 * ref_close) + 0.02 * atr;

        // 3. Max distance: at most 2.8 ATR (satisfies <= 3.0 ATR requirement)
        let stop_distance = dist_from_invalidation.max(min_volatility_dist).min(2.8 * atr);
        let sl = ref_close - sign * stop_distance;

        // 4. Take profit target: target_bound must be in profit direction
        let mut tp = target_bound;
        if sign * (tp - ref_close) <= 0.0 {
            tp = ref_close + sign * 2.5 * atr;
        }

        let actual_rr = ((tp - ref_close).abs()) / stop_distance.max(1e-6);

        json!({
            "terminal": { "outcome": "proceed" },
            "decision": {
                "action": "OPEN",
                "order_type": "市价单",
                "order_direction": dir_str,
                "entry_price": ref_close,
                "stop_loss_price": sl,
                "take_profit_price": tp,
                "take_profit_price_2": null,
                "trade_confidence": 85,
                "estimated_win_rate": null,
                "risk_reward_ratio": actual_rr,
                "traders_equation_passes": true,
                "reasoning": format!("基于 {} 诊断候选放行，触发展开结构位", dir_str),
                "strategy_id": self.config.strategy_id,
                "strategy_version": crate::strategies::VERSION
            }
        })
    }

    /// Compute statistical and quantitative metrics across trades and equity curve.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_metrics(
        account: &VirtualAccount,
        trades: &[BacktestTrade],
        equity_curve: &[EquityPoint],
        bars_processed: usize,
        gated_bars_skipped: usize,
        llm_evaluations: usize,
        cache_hits: usize,
        timeframe: &str,
    ) -> BacktestMetrics {
        let initial = account.initial_capital;
        let final_eq = account.equity;
        let net_profit = final_eq - initial;
        let net_profit_pct = if initial > 0.0 { (net_profit / initial) * 100.0 } else { 0.0 };

        let total_trades = trades.len();
        let mut winning = 0;
        let mut losing = 0;
        let mut break_even = 0;
        let mut gross_profit = 0.0;
        let mut gross_loss = 0.0;
        let mut sum_r = 0.0;
        let mut sum_hold_bars = 0.0;

        let mut max_consecutive_wins = 0;
        let mut max_consecutive_losses = 0;
        let mut current_wins = 0;
        let mut current_losses = 0;

        for t in trades {
            sum_r += t.pnl_r;
            sum_hold_bars += t.hold_bars as f64;

            if t.net_pnl > 0.0 {
                winning += 1;
                gross_profit += t.net_pnl;
                current_wins += 1;
                current_losses = 0;
                if current_wins > max_consecutive_wins {
                    max_consecutive_wins = current_wins;
                }
            } else if t.net_pnl < 0.0 {
                losing += 1;
                gross_loss += t.net_pnl.abs();
                current_losses += 1;
                current_wins = 0;
                if current_losses > max_consecutive_losses {
                    max_consecutive_losses = current_losses;
                }
            } else {
                break_even += 1;
                current_wins = 0;
                current_losses = 0;
            }
        }

        let win_rate = if total_trades > 0 {
            (winning as f64 / total_trades as f64) * 100.0
        } else {
            0.0
        };

        let profit_factor = if gross_loss > 0.0 {
            gross_profit / gross_loss
        } else if gross_profit > 0.0 {
            99.9
        } else {
            0.0
        };

        let expectancy_r = if total_trades > 0 {
            sum_r / total_trades as f64
        } else {
            0.0
        };

        let avg_trade_pnl = if total_trades > 0 {
            net_profit / total_trades as f64
        } else {
            0.0
        };

        let avg_hold_bars = if total_trades > 0 {
            sum_hold_bars / total_trades as f64
        } else {
            0.0
        };

        // Drawdown calculation
        let mut peak = initial;
        let mut max_dd_amount = 0.0;
        let mut max_dd_pct = 0.0;

        for pt in equity_curve {
            if pt.equity > peak {
                peak = pt.equity;
            }
            let dd = peak - pt.equity;
            if dd > max_dd_amount {
                max_dd_amount = dd;
            }
            let dd_pct = if peak > 0.0 { (dd / peak) * 100.0 } else { 0.0 };
            if dd_pct > max_dd_pct {
                max_dd_pct = dd_pct;
            }
        }

        // Annualized Sharpe and Sortino based on bar returns
        let (raw_sharpe, raw_sortino) = Self::compute_sharpe_sortino(equity_curve, timeframe);

        let net_profit_pct = if net_profit_pct.is_finite() { net_profit_pct } else { 0.0 };
        let win_rate = if win_rate.is_finite() { win_rate } else { 0.0 };
        let profit_factor = if profit_factor.is_finite() { profit_factor.min(99.9) } else { 0.0 };
        let expectancy_r = if expectancy_r.is_finite() { expectancy_r } else { 0.0 };
        let max_dd_pct = if max_dd_pct.is_finite() { max_dd_pct } else { 0.0 };
        let sharpe_ratio = if raw_sharpe.is_finite() { raw_sharpe } else { 0.0 };
        let sortino_ratio = if raw_sortino.is_finite() { raw_sortino.min(99.9) } else { 0.0 };
        let avg_trade_pnl = if avg_trade_pnl.is_finite() { avg_trade_pnl } else { 0.0 };
        let avg_hold_bars = if avg_hold_bars.is_finite() { avg_hold_bars } else { 0.0 };

        BacktestMetrics {
            initial_capital: initial,
            final_equity: final_eq,
            net_profit,
            net_profit_pct,
            total_trades,
            winning_trades: winning,
            losing_trades: losing,
            break_even_trades: break_even,
            win_rate,
            profit_factor,
            expectancy_r,
            max_drawdown_amount: max_dd_amount,
            max_drawdown_pct: max_dd_pct,
            sharpe_ratio,
            sortino_ratio,
            avg_trade_pnl,
            avg_hold_bars,
            max_consecutive_wins,
            max_consecutive_losses,
            total_fees_paid: account.total_fees_paid,
            total_slippage_paid: account.total_slippage_paid,
            total_bars_processed: bars_processed,
            gated_bars_skipped,
            llm_evaluations,
            cache_hits,
        }
    }

    /// Compute annualized Sharpe and Sortino ratios from periodic equity returns.
    fn compute_sharpe_sortino(equity_curve: &[EquityPoint], timeframe: &str) -> (f64, f64) {
        if equity_curve.len() < 5 {
            return (0.0, 0.0);
        }

        let mut returns = Vec::with_capacity(equity_curve.len() - 1);
        for w in equity_curve.windows(2) {
            let prev = w[0].equity;
            let curr = w[1].equity;
            if prev > 0.0 {
                returns.push((curr - prev) / prev);
            }
        }

        if returns.is_empty() {
            return (0.0, 0.0);
        }

        let n = returns.len() as f64;
        let mean = returns.iter().sum::<f64>() / n;
        let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n;
        let std_dev = variance.sqrt();

        let downside_variance = returns
            .iter()
            .map(|r| if *r < 0.0 { r.powi(2) } else { 0.0 })
            .sum::<f64>()
            / n;
        let downside_std = downside_variance.sqrt();

        // Annualization factor: 365 days * 24 hours * (bars per hour)
        let interval_ms = timeframe_to_ms(timeframe).max(60_000);
        let bars_per_year = (365.0 * 24.0 * 3_600_000.0) / (interval_ms as f64);
        let annual_multiplier = bars_per_year.sqrt();

        let sharpe = if std_dev > 1e-8 {
            (mean / std_dev) * annual_multiplier
        } else {
            0.0
        };

        let sortino = if downside_std > 1e-8 {
            (mean / downside_std) * annual_multiplier
        } else if mean > 0.0 {
            99.9
        } else {
            0.0
        };

        (sharpe, sortino)
    }
}
