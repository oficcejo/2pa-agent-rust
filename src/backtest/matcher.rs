//! Order execution simulation and walk-forward mechanical exit simulation.
//! Matches market, limit, and breakout/trigger orders, and evaluates exits using
//! conservative, pessimistic intra-bar fill modeling (stop hit before target if both occur).

use crate::backtest::account::{PositionSide, VirtualAccount};
use crate::backtest::types::BacktestTrade;
use crate::data::base::KlineBar;
use crate::learning::feedback::BarHlc;
use crate::learning::reconciler::resolve_exit;
use anyhow::Result;
use tracing::debug;

/// Pending order waiting for market fill conditions.
#[derive(Debug, Clone)]
pub struct PendingOrder {
    pub signal_id: String,
    pub strategy_id: String,
    pub symbol: String,
    pub side: PositionSide,
    pub order_type: String, // "限价单" or "突破单"
    pub target_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub contracts: f64,
    pub created_at_ms: i64,
    pub expiry_bars_remaining: u32,
}

#[derive(Debug, Clone, Default)]
pub struct OrderMatcher {
    pub pending_orders: Vec<PendingOrder>,
}

impl OrderMatcher {
    pub fn new() -> Self {
        Self {
            pending_orders: Vec::new(),
        }
    }

    /// Add a pending limit or trigger/breakout order.
    pub fn add_pending_order(&mut self, order: PendingOrder) {
        self.pending_orders.push(order);
    }

    /// Clear all pending orders.
    pub fn clear_pending(&mut self) {
        self.pending_orders.clear();
    }

    /// Try to fill pending orders against the current bar.
    /// Returns any orders that were successfully filled and opened.
    pub fn process_pending_orders(
        &mut self,
        account: &mut VirtualAccount,
        bar: &KlineBar,
    ) -> Vec<String> {
        let mut filled_signals = Vec::new();
        let mut still_pending = Vec::new();

        for mut order in self.pending_orders.drain(..) {
            if account.position.is_some() || account.is_liquidated {
                // Cannot open another position if already in position
                still_pending.push(order);
                continue;
            }

            let mut filled = false;
            let mut fill_price = order.target_price;

            match order.order_type.as_str() {
                "限价单" => match order.side {
                    PositionSide::Long => {
                        if bar.low <= order.target_price {
                            filled = true;
                            // Pessimistic fill: limit price or bar open if opened lower
                            fill_price = order.target_price.min(bar.open);
                        }
                    }
                    PositionSide::Short => {
                        if bar.high >= order.target_price {
                            filled = true;
                            fill_price = order.target_price.max(bar.open);
                        }
                    }
                },
                "突破单" => match order.side {
                    PositionSide::Long => {
                        if bar.high >= order.target_price {
                            filled = true;
                            // Pessimistic fill: order target or bar open if opened higher
                            fill_price = order.target_price.max(bar.open);
                        }
                    }
                    PositionSide::Short => {
                        if bar.low <= order.target_price {
                            filled = true;
                            // Pessimistic fill: order target or bar open if opened lower
                            fill_price = order.target_price.min(bar.open);
                        }
                    }
                },
                _ => {}
            }

            if filled {
                debug!(
                    "Pending {} filled for signal {} at price {}",
                    order.order_type, order.signal_id, fill_price
                );
                if let Ok(()) = account.open_position(
                    &order.strategy_id,
                    &order.signal_id,
                    &order.symbol,
                    order.side,
                    &order.order_type,
                    fill_price,
                    order.stop_loss,
                    order.take_profit,
                    order.contracts,
                    bar.ts_open,
                ) {
                    filled_signals.push(order.signal_id);
                }
            } else {
                order.expiry_bars_remaining = order.expiry_bars_remaining.saturating_sub(1);
                if order.expiry_bars_remaining > 0 {
                    still_pending.push(order);
                } else {
                    debug!("Pending order expired for signal {}", order.signal_id);
                }
            }
        }

        self.pending_orders = still_pending;
        filled_signals
    }

    /// Check if active position hits mechanical exit criteria (SL, TP, timeout, liquidation).
    /// Uses conservative pessimistic modeling (stop loss checked before take profit).
    pub fn check_mechanical_exit(
        &self,
        account: &mut VirtualAccount,
        bar: &KlineBar,
        max_hold_bars: u32,
    ) -> Result<Option<BacktestTrade>> {
        if account.position.is_none() {
            return Ok(None);
        }

        let pos = account.position.as_ref().unwrap();
        let liq_price_opt = account.liquidation_price();

        // 1. Check if bar opened directly in liquidation territory (gap on open past maintenance margin)
        let opened_in_liquidation = match (pos.side.clone(), liq_price_opt) {
            (PositionSide::Long, Some(liq)) => bar.open <= liq,
            (PositionSide::Short, Some(liq)) => bar.open >= liq,
            _ => false,
        };
        if opened_in_liquidation {
            account.is_liquidated = true;
            let trade = account.close_position(bar.open, bar.ts_open, "liquidated")?;
            return Ok(Some(trade));
        }

        // Convert current bar to BarHlc to leverage reconciler logic
        let bar_hlc = BarHlc {
            high: bar.high,
            low: bar.low,
            close: bar.close,
        };

        let side_str = pos.side.as_str();
        let hold_bars = pos.hold_bars;

        // Intra-bar check using pessimistic logic (SL evaluated before TP)
        let resolved = resolve_exit(
            side_str,
            pos.entry_price,
            pos.stop_loss,
            pos.take_profit,
            &[bar_hlc],
            0, // do not expire on 1-bar slice
        );

        if let Some(res) = resolved {
            let reason_str = res.exit_reason.as_str();
            let mut exit_price = res.exit_price;

            // Check if stop loss was hit
            if reason_str == "stop_loss" {
                // If stop loss is placed beyond liquidation price, liquidation triggers first!
                let liq_hit_before_sl = match (pos.side.clone(), liq_price_opt) {
                    (PositionSide::Long, Some(liq)) => pos.stop_loss < liq,
                    (PositionSide::Short, Some(liq)) => pos.stop_loss > liq,
                    _ => false,
                };
                if liq_hit_before_sl {
                    account.is_liquidated = true;
                    let trade = account.close_position(liq_price_opt.unwrap(), bar.ts_open, "liquidated")?;
                    return Ok(Some(trade));
                }

                // Pessimistic gap fill: if candle opened beyond stop loss, fill at bar.open
                match pos.side {
                    PositionSide::Long => {
                        if bar.open < pos.stop_loss {
                            exit_price = bar.open;
                        }
                    }
                    PositionSide::Short => {
                        if bar.open > pos.stop_loss {
                            exit_price = bar.open;
                        }
                    }
                }
            }

            let trade = account.close_position(exit_price, bar.ts_open, reason_str)?;
            return Ok(Some(trade));
        }

        // 2. Check if price touched liquidation price intra-bar without a preceding SL
        if let Some(liq) = liq_price_opt {
            let liquidated_intrabar = match pos.side {
                PositionSide::Long => bar.low <= liq,
                PositionSide::Short => bar.high >= liq,
            };
            if liquidated_intrabar {
                account.is_liquidated = true;
                let trade = account.close_position(liq, bar.ts_open, "liquidated")?;
                return Ok(Some(trade));
            }
        }

        // 3. Check hold bars limit expiration
        if max_hold_bars > 0 && hold_bars >= max_hold_bars {
            let trade = account.close_position(bar.close, bar.ts_open, "expired")?;
            return Ok(Some(trade));
        }

        Ok(None)
    }
}
