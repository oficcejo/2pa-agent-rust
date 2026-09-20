use crate::data::base::KlineBar;
use crate::learning::feedback::TradeOutcome;
use crate::records::benchmark::{save_benchmark_episode, BenchmarkEpisode};
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Converter that solidifies settled live or shadow trade outcomes into benchmark episodes.
///
/// Implements the GEPA data flywheel:
/// When real or shadow trades settle, rather than being discarded or only stored in logs,
/// they can be converted into standalone, reproducible historical benchmark slices
/// to expand the offline evaluation suite.
pub struct Trade2Episode;

impl Trade2Episode {
    /// Convert a resolved TradeOutcome into a BenchmarkEpisode.
    pub fn convert(
        outcome: &TradeOutcome,
        kline_data: Vec<KlineBar>,
        future_bars: Vec<KlineBar>,
    ) -> BenchmarkEpisode {
        let is_win = outcome.r_multiple > 0.0;
        let side_lower = outcome.side.to_lowercase();
        let is_long = side_lower.contains("long") || side_lower.contains("多") || side_lower.contains("buy");
        let is_short = side_lower.contains("short") || side_lower.contains("空") || side_lower.contains("sell");

        let expected_action = if is_win {
            if is_long {
                "OPEN_LONG"
            } else if is_short {
                "OPEN_SHORT"
            } else {
                "WAIT"
            }
        } else {
            // A losing trade signifies that the system should have waited or stayed out
            "WAIT"
        };

        let sanitized_symbol = outcome.symbol.replace("/", "-").replace(":", "-");
        let episode_id = format!(
            "flywheel_{}_{}",
            sanitized_symbol.to_lowercase(),
            &outcome.signal_id[..outcome.signal_id.len().min(12)]
        );

        let description = format!(
            "实盘对账自动固化基准切片: 标的 {} | 周期 {} | 信号 {} | 实际盈亏 {:.2}R ({})",
            outcome.symbol,
            outcome.timeframe,
            outcome.signal_id,
            outcome.r_multiple,
            outcome.exit_reason
        );

        let mut final_future_bars = future_bars;
        if final_future_bars.is_empty() && outcome.filled && outcome.entry_price > 0.0 {
            let exit_px = outcome.exit_price.unwrap_or(outcome.entry_price);
            let n_bars = outcome.hold_bars.max(1) as usize;
            let start_ts = outcome.created_ms;
            let end_ts = if outcome.resolved_ms > start_ts { outcome.resolved_ms } else { start_ts + (n_bars as i64) * 900_000 };
            let ts_step = ((end_ts - start_ts) / (n_bars as i64)).max(1000);
            let risk = (outcome.entry_price - outcome.stop_price).abs().max(1.0);

            for i in 0..n_bars {
                let bar_ts = start_ts + (i as i64) * ts_step;
                let is_last = i == n_bars - 1;
                let progress = (i as f64) / (n_bars as f64);
                let current_px = outcome.entry_price + (exit_px - outcome.entry_price) * progress;
                let close_px = if is_last { exit_px } else { current_px };
                
                let (high, low) = if is_long {
                    let h = current_px + (outcome.mfe_r * risk).min(risk * 3.0);
                    let l = current_px - (outcome.mae_r * risk).min(risk * 2.0);
                    (h.max(close_px).max(current_px), l.min(close_px).min(current_px))
                } else {
                    let h = current_px + (outcome.mae_r * risk).min(risk * 2.0);
                    let l = current_px - (outcome.mfe_r * risk).min(risk * 3.0);
                    (h.max(close_px).max(current_px), l.min(close_px).min(current_px))
                };

                final_future_bars.push(KlineBar {
                    seq: (i + 1),
                    ts_open: bar_ts,
                    open: current_px,
                    high,
                    low,
                    close: close_px,
                    volume: 100.0,
                    amount: 0.0,
                    pct_chg: None,
                    closed: true,
                });
            }
        }

        let benchmark_r = if is_win { outcome.r_multiple } else { 0.0 };

        BenchmarkEpisode {
            episode_id,
            symbol: outcome.symbol.clone(),
            timeframe: outcome.timeframe.clone(),
            market_regime: outcome.cycle_position.clone(),
            description,
            kline_data,
            htf_text: None,
            expected_action: expected_action.to_string(),
            future_bars: final_future_bars,
            benchmark_r,
        }
    }

    /// Solidify a settled trade directly to the benchmark episodes folder on disk.
    pub fn solidify(
        benchmark_dir: &Path,
        outcome: &TradeOutcome,
        kline_data: Vec<KlineBar>,
        future_bars: Vec<KlineBar>,
    ) -> Result<PathBuf> {
        let episode = Self::convert(outcome, kline_data, future_bars);
        save_benchmark_episode(benchmark_dir, &episode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_outcome(r: f64) -> TradeOutcome {
        TradeOutcome {
            signal_id: "sig_live_999".to_string(),
            decision_record_id: "rec_live_999".to_string(),
            strategy_id: "2pa_trend".to_string(),
            strategy_version: "v1".to_string(),
            prompt_version: "v1".to_string(),
            prompt_hash: "hash".to_string(),
            symbol: "ETH-USDT-SWAP".to_string(),
            timeframe: "15m".to_string(),
            side: "long".to_string(),
            cycle_position: "tight_channel".to_string(),
            detected_patterns: vec!["h2".to_string()],
            entry_price: 3000.0,
            stop_price: 2950.0,
            target_price: 3100.0,
            exit_price: Some(3100.0),
            size: 1.0,
            filled: true,
            fill_ratio: 1.0,
            fees_usd: 2.0,
            realized_pnl_usd: 150.0,
            pnl_source: "broker".to_string(),
            r_multiple: r,
            mfe_r: 2.0,
            mae_r: 0.2,
            hold_bars: 8,
            exit_reason: "take_profit".to_string(),
            qualified: true,
            qualification_reason: "实盘对账完全合格".to_string(),
            created_ms: 1000,
            resolved_ms: 2000,
        }
    }

    #[test]
    fn test_trade2episode_converts_winning_trade() {
        let outcome = dummy_outcome(2.0);
        let ep = Trade2Episode::convert(&outcome, vec![], vec![]);

        assert_eq!(ep.expected_action, "OPEN_LONG");
        assert_eq!(ep.benchmark_r, 2.0);
        assert!(ep.episode_id.contains("flywheel_eth-usdt-swap"));
        assert_eq!(ep.market_regime, "tight_channel");
    }

    #[test]
    fn test_trade2episode_converts_losing_trade_to_wait() {
        let outcome = dummy_outcome(-1.0);
        let ep = Trade2Episode::convert(&outcome, vec![], vec![]);

        // Losing trade expected action is WAIT
        assert_eq!(ep.expected_action, "WAIT");
        assert_eq!(ep.benchmark_r, 0.0);
    }
}
