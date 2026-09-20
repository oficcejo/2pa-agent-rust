//! Realistic virtual margin account simulating OKX contract specifications.
//! Handles margin requirements, leverage, lot sizing (lotSz, ctVal), liquidation,
//! taker/maker fees, and conservative slippage.

use crate::backtest::types::{BacktestConfig, BacktestTrade};
use anyhow::{anyhow, ensure, Result};
use tracing::debug;

#[derive(Debug, Clone, PartialEq)]
pub enum PositionSide {
    Long,
    Short,
}

impl PositionSide {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Long => "做多",
            Self::Short => "做空",
        }
    }

    pub fn sign(&self) -> f64 {
        match self {
            Self::Long => 1.0,
            Self::Short => -1.0,
        }
    }
}

/// Active virtual contract position.
#[derive(Debug, Clone)]
pub struct VirtualPosition {
    pub symbol: String,
    pub side: PositionSide,
    pub contracts: f64,       // In integer lots
    pub coins: f64,           // contracts * ct_val
    pub entry_price: f64,     // Execution price including slippage
    pub requested_entry: f64, // Nominal proposed entry price
    pub stop_loss: f64,
    pub take_profit: f64,
    pub margin: f64,          // Initial margin locked
    pub entry_time_ms: i64,
    pub hold_bars: u32,
    pub mfe_r: f64,
    pub mae_r: f64,
    pub signal_id: String,
    pub strategy_id: String,
    pub order_type: String,
    pub entry_fee: f64,
    pub entry_slippage: f64,
    pub initial_risk_usdt: f64,
}

/// Virtual margin account tracking balances, margin, and trades.
#[derive(Debug, Clone)]
pub struct VirtualAccount {
    pub initial_capital: f64,
    pub cash: f64,
    pub equity: f64,
    pub ct_val: f64,
    pub lot_sz: f64,
    pub leverage: f64,
    pub maintenance_margin_ratio: f64,
    pub taker_fee_rate: f64,
    pub maker_fee_rate: f64,
    pub slippage_rate: f64,
    pub position: Option<VirtualPosition>,
    pub total_fees_paid: f64,
    pub total_slippage_paid: f64,
    pub is_liquidated: bool,
    pub total_trades_closed: usize,
}

impl VirtualAccount {
    pub fn new(config: &BacktestConfig) -> Self {
        Self {
            initial_capital: config.initial_capital,
            cash: config.initial_capital,
            equity: config.initial_capital,
            ct_val: config.ct_val.max(0.00001),
            lot_sz: config.lot_sz.max(1.0),
            leverage: config.leverage.max(1.0),
            maintenance_margin_ratio: 0.01, // 1% maintenance margin
            taker_fee_rate: config.taker_fee_rate,
            maker_fee_rate: config.maker_fee_rate,
            slippage_rate: config.slippage_rate,
            position: None,
            total_fees_paid: 0.0,
            total_slippage_paid: 0.0,
            is_liquidated: false,
            total_trades_closed: 0,
        }
    }

    /// Calculate order sizing in lots (contracts) respecting risk percent and margin budget.
    pub fn calculate_order_size(
        &self,
        entry: f64,
        stop: f64,
        risk_pct: f64,
        max_margin_pct: f64,
    ) -> f64 {
        if entry <= 0.0 || stop <= 0.0 || (entry - stop).abs() <= 1e-6 || self.cash <= 0.0 {
            return 0.0;
        }

        // Risk per single contract (ct_val coins)
        let price_diff = (entry - stop).abs();
        let notional_per_lot = entry * self.ct_val;
        let round_trip_fees_per_lot = notional_per_lot * self.taker_fee_rate * 2.0;
        let risk_per_lot = price_diff * self.ct_val + round_trip_fees_per_lot;

        if risk_per_lot <= 0.0 {
            return 0.0;
        }

        // 1. Risk budget cap
        let risk_budget = self.equity * (risk_pct / 100.0);
        let max_lots_by_risk = risk_budget / risk_per_lot;

        // 2. Margin budget cap
        let margin_budget = self.cash.min(self.equity * (max_margin_pct / 100.0));
        let margin_per_lot = notional_per_lot / self.leverage + notional_per_lot * self.taker_fee_rate;

        if margin_per_lot <= 0.0 {
            return 0.0;
        }

        let max_lots_by_margin = margin_budget / margin_per_lot;

        // Final lots clamped and floored by lot_sz
        let target_lots = max_lots_by_risk.min(max_lots_by_margin);
        let floored_lots = (target_lots / self.lot_sz).floor() * self.lot_sz;

        if floored_lots < self.lot_sz {
            0.0
        } else {
            floored_lots
        }
    }

    /// Open a new position with pessimistic slippage and taker fee deduction.
    #[allow(clippy::too_many_arguments)]
    pub fn open_position(
        &mut self,
        strategy_id: &str,
        signal_id: &str,
        symbol: &str,
        side: PositionSide,
        order_type: &str,
        requested_entry: f64,
        stop_loss: f64,
        take_profit: f64,
        contracts: f64,
        timestamp_ms: i64,
    ) -> Result<()> {
        ensure!(!self.is_liquidated, "账户已被强平，禁止新开仓");
        ensure!(self.position.is_none(), "已有持仓，禁止重复开仓");
        ensure!(contracts >= self.lot_sz, "开仓张数小于最小下单步长 {}", self.lot_sz);

        // Pessimistic execution price with slippage:
        // Buying (Long) pays more; Selling (Short) gets less
        let entry_price = match side {
            PositionSide::Long => requested_entry * (1.0 + self.slippage_rate),
            PositionSide::Short => requested_entry * (1.0 - self.slippage_rate),
        };

        let coins = contracts * self.ct_val;
        let notional = coins * entry_price;
        let initial_margin = notional / self.leverage;
        let entry_fee = notional * self.taker_fee_rate;
        let entry_slippage = (entry_price - requested_entry).abs() * coins;

        let total_required = initial_margin + entry_fee;
        ensure!(
            self.cash >= total_required,
            "现金不足：需要 {} USDT (保证金 {} + 手续费 {})，当前可用 {} USDT",
            total_required,
            initial_margin,
            entry_fee,
            self.cash
        );

        // Deduct margin and fee from cash
        self.cash -= total_required;
        self.total_fees_paid += entry_fee;
        self.total_slippage_paid += entry_slippage;

        let initial_risk_usdt = ((entry_price - stop_loss).abs() * coins + entry_fee * 2.0).max(1e-6);

        self.position = Some(VirtualPosition {
            symbol: symbol.to_string(),
            side,
            contracts,
            coins,
            entry_price,
            requested_entry,
            stop_loss,
            take_profit,
            margin: initial_margin,
            entry_time_ms: timestamp_ms,
            hold_bars: 0,
            mfe_r: 0.0,
            mae_r: 0.0,
            signal_id: signal_id.to_string(),
            strategy_id: strategy_id.to_string(),
            order_type: order_type.to_string(),
            entry_fee,
            entry_slippage,
            initial_risk_usdt,
        });

        self.update_equity(entry_price);
        Ok(())
    }

    /// Calculate the exact liquidation price for the active position.
    pub fn liquidation_price(&self) -> Option<f64> {
        let pos = self.position.as_ref()?;
        if pos.coins <= 0.0 {
            return None;
        }
        let total_collateral = self.cash + pos.margin;
        let mmr = self.maintenance_margin_ratio;
        match pos.side {
            PositionSide::Long => {
                let denom = pos.coins * (1.0 - mmr);
                if denom <= 0.0 {
                    None
                } else {
                    let num = pos.coins * pos.entry_price - total_collateral;
                    Some((num / denom).max(0.0))
                }
            }
            PositionSide::Short => {
                let denom = pos.coins * (1.0 + mmr);
                if denom <= 0.0 {
                    None
                } else {
                    let num = total_collateral + pos.coins * pos.entry_price;
                    Some(num / denom)
                }
            }
        }
    }

    /// Update position excursion and account equity based on current bar.
    pub fn update_bar(&mut self, high: f64, low: f64, close: f64) {
        if self.position.is_none() {
            self.equity = self.cash;
            return;
        }

        {
            let pos = self.position.as_mut().unwrap();
            pos.hold_bars += 1;

            let risk = (pos.entry_price - pos.stop_loss).abs();
            if risk > 1e-6 {
                let (favorable_price, adverse_price) = match pos.side {
                    PositionSide::Long => (high, low),
                    PositionSide::Short => (low, high),
                };

                let current_mfe = ((favorable_price - pos.entry_price) * pos.side.sign()) / risk;
                let current_mae = ((pos.entry_price - adverse_price) * pos.side.sign()) / risk;

                pos.mfe_r = pos.mfe_r.max(current_mfe).max(0.0);
                pos.mae_r = pos.mae_r.max(current_mae).max(0.0);
            }
        }

        self.update_equity(close);
    }

    /// Calculate unrealized PnL for active position.
    pub fn unrealized_pnl(&self, current_price: f64) -> f64 {
        match &self.position {
            Some(pos) => match pos.side {
                PositionSide::Long => pos.coins * (current_price - pos.entry_price),
                PositionSide::Short => pos.coins * (pos.entry_price - current_price),
            },
            None => 0.0,
        }
    }

    /// Mark to market: recalculate total equity.
    pub fn update_equity(&mut self, current_price: f64) {
        let upl = self.unrealized_pnl(current_price);
        let margin = self.position.as_ref().map(|p| p.margin).unwrap_or(0.0);
        self.equity = (self.cash + margin + upl).max(0.0);
    }

    /// Close active position with pessimistic exit slippage and taker fee.
    pub fn close_position(
        &mut self,
        nominal_exit_price: f64,
        exit_time_ms: i64,
        exit_reason: &str,
    ) -> Result<BacktestTrade> {
        let pos = self.position.take().ok_or_else(|| anyhow!("无持仓可平"))?;

        // Exit slippage: selling gets less; buying covers higher
        let actual_exit_price = match pos.side {
            PositionSide::Long => nominal_exit_price * (1.0 - self.slippage_rate),
            PositionSide::Short => nominal_exit_price * (1.0 + self.slippage_rate),
        };

        let exit_notional = pos.coins * actual_exit_price;
        let exit_fee = exit_notional * self.taker_fee_rate;
        let exit_slippage = (actual_exit_price - nominal_exit_price).abs() * pos.coins;

        self.total_fees_paid += exit_fee;
        self.total_slippage_paid += exit_slippage;

        let gross_pnl = match pos.side {
            PositionSide::Long => pos.coins * (actual_exit_price - pos.entry_price),
            PositionSide::Short => pos.coins * (pos.entry_price - actual_exit_price),
        };

        // True net PnL includes BOTH entry and exit fees
        let net_pnl = gross_pnl - pos.entry_fee - exit_fee;
        let total_fees = pos.entry_fee + exit_fee;
        let total_slippage = pos.entry_slippage + exit_slippage;

        // Return margin and net PnL to cash (entry_fee was deducted at open, so add back margin + pos.entry_fee + net_pnl)
        self.cash += pos.margin + pos.entry_fee + net_pnl;
        if self.is_liquidated || exit_reason == "liquidated" {
            self.cash = self.cash.max(0.0);
            self.is_liquidated = true;
        }
        self.equity = self.cash.max(0.0);
        self.total_trades_closed += 1;

        let pnl_percent = if pos.margin > 0.0 {
            (net_pnl / pos.margin) * 100.0
        } else {
            0.0
        };

        let pnl_r = if pos.initial_risk_usdt > 0.0 {
            net_pnl / pos.initial_risk_usdt
        } else {
            0.0
        };

        let trade = BacktestTrade {
            trade_id: format!("TRD-{}-{}", exit_time_ms, self.total_trades_closed),
            signal_id: pos.signal_id,
            strategy_id: pos.strategy_id,
            direction: pos.side.as_str().to_string(),
            order_type: pos.order_type,
            entry_time_ms: pos.entry_time_ms,
            entry_price: pos.entry_price,
            exit_time_ms,
            exit_price: actual_exit_price,
            contracts: pos.contracts,
            notional_usdt: exit_notional,
            stop_loss: pos.stop_loss,
            take_profit: pos.take_profit,
            gross_pnl,
            net_pnl,
            pnl_percent,
            pnl_r,
            exit_reason: exit_reason.to_string(),
            fees: total_fees,
            slippage: total_slippage,
            mfe_r: pos.mfe_r,
            mae_r: pos.mae_r,
            hold_bars: pos.hold_bars,
            notes: format!(
                "开仓均价: {:.2} (请求 {:.2}), 平仓均价: {:.2} (请求 {:.2})",
                pos.entry_price, pos.requested_entry, actual_exit_price, nominal_exit_price
            ),
        };

        debug!(
            "Closed trade {}: direction={}, pnl={:.2} USDT ({:.2}R), reason={}",
            trade.trade_id, trade.direction, trade.net_pnl, trade.pnl_r, trade.exit_reason
        );

        Ok(trade)
    }
}
