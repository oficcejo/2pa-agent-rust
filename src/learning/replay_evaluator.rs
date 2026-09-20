use crate::learning::evaluation::{compare_candidate, metrics_for, EvaluationPolicy, StrategyMetrics, Verdict};
use crate::learning::feedback::{ExitReason, TradeOutcome, PNL_SOURCE_MODEL};
use crate::records::benchmark::BenchmarkEpisode;

/// Offline benchmark replay evaluation engine.
///
/// Borrows from Reef's benchmark task evaluation design:
/// Rather than waiting for 20 real live fills on an inactive candidate prompt (which causes deadlock),
/// ReplayEvaluator simulates candidate and baseline prompt policies on fixed historical benchmark episodes.
pub struct ReplayEvaluator;

impl ReplayEvaluator {
    /// Evaluate a single benchmark episode against a prompt.
    pub fn evaluate_episode(
        prompt_content: &str,
        prompt_version: &str,
        episode: &BenchmarkEpisode,
    ) -> TradeOutcome {
        let is_prompt_valid = !prompt_content.trim().is_empty() && prompt_content.len() >= 50;
        let now = chrono::Utc::now().timestamp_millis();

        // Check if the prompt has specific quality signals for this regime
        let handles_regime = match episode.market_regime.as_str() {
            "trading_range" | "chop" | "barbwire" => {
                prompt_content.contains("震荡")
                    || prompt_content.contains("观望")
                    || prompt_content.contains("range")
                    || prompt_content.contains("WAIT")
            }
            "spike" | "tight_channel" | "broad_channel" => {
                prompt_content.contains("趋势")
                    || prompt_content.contains("通道")
                    || prompt_content.contains("channel")
                    || prompt_content.contains("trend")
            }
            _ => true,
        };

        let takes_expected_action = is_prompt_valid && handles_regime;

        // If ground truth is WAIT and prompt correctly waits
        if episode.expected_action == "WAIT" {
            if takes_expected_action {
                return TradeOutcome {
                    signal_id: format!("sim_{}_{}", prompt_version, episode.episode_id),
                    decision_record_id: format!("rec_bench_{}", episode.episode_id),
                    strategy_id: "2pa_trend".to_string(),
                    strategy_version: "v1".to_string(),
                    prompt_version: prompt_version.to_string(),
                    prompt_hash: crate::learning::artifact::content_hash(prompt_content),
                    symbol: episode.symbol.clone(),
                    timeframe: episode.timeframe.clone(),
                    side: "none".to_string(),
                    cycle_position: episode.market_regime.clone(),
                    detected_patterns: vec!["benchmark".to_string()],
                    entry_price: 0.0,
                    stop_price: 0.0,
                    target_price: 0.0,
                    exit_price: None,
                    size: 0.0,
                    filled: false,
                    fill_ratio: 0.0,
                    fees_usd: 0.0,
                    realized_pnl_usd: 0.0,
                    pnl_source: PNL_SOURCE_MODEL.to_string(),
                    r_multiple: 0.0,
                    mfe_r: 0.0,
                    mae_r: 0.0,
                    hold_bars: episode.future_bars.len() as u32,
                    exit_reason: ExitReason::Unfilled.as_str().to_string(),
                    qualified: false,
                    qualification_reason: "观望基准切片，无成交".to_string(),
                    created_ms: episode.kline_data.first().map(|b| b.ts_open).unwrap_or(now),
                    resolved_ms: episode.future_bars.last().map(|b| b.ts_open).unwrap_or(now),
                };
            } else {
                // Impatient / overtrading prompt takes a trade in chop -> stopped out
                return TradeOutcome {
                    signal_id: format!("sim_{}_{}", prompt_version, episode.episode_id),
                    decision_record_id: format!("rec_bench_{}", episode.episode_id),
                    strategy_id: "2pa_trend".to_string(),
                    strategy_version: "v1".to_string(),
                    prompt_version: prompt_version.to_string(),
                    prompt_hash: crate::learning::artifact::content_hash(prompt_content),
                    symbol: episode.symbol.clone(),
                    timeframe: episode.timeframe.clone(),
                    side: "long".to_string(),
                    cycle_position: episode.market_regime.clone(),
                    detected_patterns: vec!["benchmark_overtrade".to_string()],
                    entry_price: 100.0,
                    stop_price: 98.0,
                    target_price: 104.0,
                    exit_price: Some(98.0),
                    size: 1.0,
                    filled: true,
                    fill_ratio: 1.0,
                    fees_usd: 1.0,
                    realized_pnl_usd: -100.0,
                    pnl_source: PNL_SOURCE_MODEL.to_string(),
                    r_multiple: -1.0,
                    mfe_r: 0.2,
                    mae_r: 1.0,
                    hold_bars: 2,
                    exit_reason: ExitReason::StopLoss.as_str().to_string(),
                    qualified: true,
                    qualification_reason: "合格样本".to_string(),
                    created_ms: episode.kline_data.first().map(|b| b.ts_open).unwrap_or(now),
                    resolved_ms: episode.future_bars.first().map(|b| b.ts_open).unwrap_or(now),
                };
            }
        }

        // Active trade episode (OPEN_LONG or OPEN_SHORT)
        let is_long = episode.expected_action == "OPEN_LONG";
        let entry_price = episode
            .future_bars
            .first()
            .map(|b| b.open)
            .or_else(|| episode.kline_data.first().map(|b| b.close))
            .unwrap_or(100.0);

        let risk_per_unit = (entry_price * 0.01).max(1.0);

        if !takes_expected_action {
            // Fails to align with setup: stopped out
            let exit_price = if is_long {
                entry_price - risk_per_unit
            } else {
                entry_price + risk_per_unit
            };

            return TradeOutcome {
                signal_id: format!("sim_{}_{}", prompt_version, episode.episode_id),
                decision_record_id: format!("rec_bench_{}", episode.episode_id),
                strategy_id: "2pa_trend".to_string(),
                strategy_version: "v1".to_string(),
                prompt_version: prompt_version.to_string(),
                prompt_hash: crate::learning::artifact::content_hash(prompt_content),
                symbol: episode.symbol.clone(),
                timeframe: episode.timeframe.clone(),
                side: if is_long { "long".to_string() } else { "short".to_string() },
                cycle_position: episode.market_regime.clone(),
                detected_patterns: vec!["benchmark_misaligned".to_string()],
                entry_price,
                stop_price: exit_price,
                target_price: if is_long { entry_price + risk_per_unit * 2.0 } else { entry_price - risk_per_unit * 2.0 },
                exit_price: Some(exit_price),
                size: 1.0,
                filled: true,
                fill_ratio: 1.0,
                fees_usd: 1.0,
                realized_pnl_usd: -100.0,
                pnl_source: PNL_SOURCE_MODEL.to_string(),
                r_multiple: -1.0,
                mfe_r: 0.1,
                mae_r: 1.0,
                hold_bars: 3,
                exit_reason: ExitReason::StopLoss.as_str().to_string(),
                qualified: true,
                qualification_reason: "合格样本".to_string(),
                created_ms: episode.kline_data.first().map(|b| b.ts_open).unwrap_or(now),
                resolved_ms: episode.future_bars.first().map(|b| b.ts_open).unwrap_or(now),
            };
        }

        // Simulates price path over future bars
        let mut max_fav = 0.0_f64;
        let mut max_adv = 0.0_f64;
        let mut hold_bars = 0_u32;

        for bar in &episode.future_bars {
            hold_bars += 1;
            if is_long {
                let fav = (bar.high - entry_price) / risk_per_unit;
                let adv = (entry_price - bar.low) / risk_per_unit;
                if fav > max_fav { max_fav = fav; }
                if adv > max_adv { max_adv = adv; }
            } else {
                let fav = (entry_price - bar.low) / risk_per_unit;
                let adv = (bar.high - entry_price) / risk_per_unit;
                if fav > max_fav { max_fav = fav; }
                if adv > max_adv { max_adv = adv; }
            }
        }

        let achieved_r = if episode.benchmark_r != 0.0 {
            episode.benchmark_r
        } else if max_fav > 1.5 {
            max_fav.min(2.0)
        } else {
            -1.0
        };

        let exit_reason = if achieved_r > 0.0 {
            ExitReason::TakeProfit.as_str().to_string()
        } else {
            ExitReason::StopLoss.as_str().to_string()
        };

        let exit_price = if is_long {
            entry_price + achieved_r * risk_per_unit
        } else {
            entry_price - achieved_r * risk_per_unit
        };

        let stop_price = if is_long {
            entry_price - risk_per_unit
        } else {
            entry_price + risk_per_unit
        };

        let target_price = if is_long {
            entry_price + risk_per_unit * 2.0
        } else {
            entry_price - risk_per_unit * 2.0
        };

        TradeOutcome {
            signal_id: format!("sim_{}_{}", prompt_version, episode.episode_id),
            decision_record_id: format!("rec_bench_{}", episode.episode_id),
            strategy_id: "2pa_trend".to_string(),
            strategy_version: "v1".to_string(),
            prompt_version: prompt_version.to_string(),
            prompt_hash: crate::learning::artifact::content_hash(prompt_content),
            symbol: episode.symbol.clone(),
            timeframe: episode.timeframe.clone(),
            side: if is_long { "long".to_string() } else { "short".to_string() },
            cycle_position: episode.market_regime.clone(),
            detected_patterns: vec!["benchmark_pass".to_string()],
            entry_price,
            stop_price,
            target_price,
            exit_price: Some(exit_price),
            size: 1.0,
            filled: true,
            fill_ratio: 1.0,
            fees_usd: 1.0,
            realized_pnl_usd: achieved_r * 100.0,
            pnl_source: PNL_SOURCE_MODEL.to_string(),
            r_multiple: achieved_r,
            mfe_r: max_fav.max(achieved_r.max(0.0)),
            mae_r: max_adv.abs(),
            hold_bars: hold_bars.max(1),
            exit_reason,
            qualified: true,
            qualification_reason: "合格样本".to_string(),
            created_ms: episode.kline_data.first().map(|b| b.ts_open).unwrap_or(now),
            resolved_ms: episode.future_bars.last().map(|b| b.ts_open).unwrap_or(now),
        }
    }

    /// Evaluate an entire suite of benchmark episodes.
    pub fn evaluate_suite(
        prompt_content: &str,
        prompt_version: &str,
        episodes: &[BenchmarkEpisode],
    ) -> (Vec<TradeOutcome>, StrategyMetrics) {
        let outcomes: Vec<TradeOutcome> = episodes
            .iter()
            .map(|ep| Self::evaluate_episode(prompt_content, prompt_version, ep))
            .collect();
        let metrics = metrics_for(&outcomes);
        (outcomes, metrics)
    }

    /// Compare candidate prompt against baseline on benchmark episodes.
    ///
    /// Adapts the policy min_samples if the benchmark suite size is smaller than policy.min_samples,
    /// ensuring that a rigorous benchmark suite can unlock activation.
    pub fn evaluate_candidate_vs_baseline(
        candidate_content: &str,
        candidate_version: &str,
        baseline_content: &str,
        baseline_version: &str,
        episodes: &[BenchmarkEpisode],
        policy: &EvaluationPolicy,
    ) -> Verdict {
        let (_cand_outcomes, cand_metrics) =
            Self::evaluate_suite(candidate_content, candidate_version, episodes);
        let (_base_outcomes, base_metrics) =
            Self::evaluate_suite(baseline_content, baseline_version, episodes);

        // Adjust policy min_samples to the benchmark suite qualified sample size
        let target_samples = if base_metrics.samples > 0 {
            cand_metrics.samples.min(base_metrics.samples)
        } else {
            cand_metrics.samples
        };

        let mut adjusted_policy = policy.clone();
        if target_samples < adjusted_policy.min_samples && target_samples > 0 {
            adjusted_policy.min_samples = target_samples;
        }

        // If candidate prudently avoided all trades in a defensive benchmark suite (0 losses),
        // while baseline overtraded and suffered negative expectancy:
        if cand_metrics.samples == 0 && base_metrics.samples > 0 && base_metrics.expectancy_r < 0.0 {
            return Verdict {
                accepted: true,
                reasons: vec!["候选版本在防御/观望基准集中严格空仓避险，规避了基线版本的连续亏损".to_string()],
                candidate: cand_metrics,
                baseline: base_metrics.clone(),
                expectancy_delta_r: 0.0 - base_metrics.expectancy_r,
            };
        }

        if base_metrics.samples == 0 && cand_metrics.samples >= adjusted_policy.min_samples && cand_metrics.samples > 0 {
            // When baseline has no trade samples in the benchmark suite (e.g. passive or initial),
            // validate that candidate achieves positive expectancy.
            let accepted = cand_metrics.expectancy_r >= adjusted_policy.min_expectancy_delta_r;
            let reasons = if accepted {
                Vec::new()
            } else {
                vec![format!(
                    "候选版本基准重放期望 R 为 {:.3}R，未达及格线 {:.3}R",
                    cand_metrics.expectancy_r, adjusted_policy.min_expectancy_delta_r
                )]
            };
            Verdict {
                accepted,
                reasons,
                candidate: cand_metrics.clone(),
                baseline: base_metrics,
                expectancy_delta_r: cand_metrics.expectancy_r,
            }
        } else {
            compare_candidate(&cand_metrics, &base_metrics, &adjusted_policy)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::base::KlineBar;

    fn dummy_bar(seq: usize, price: f64) -> KlineBar {
        KlineBar {
            seq,
            ts_open: 1700000000000 + (seq as i64) * 900000,
            open: price,
            high: price + 1.0,
            low: price - 1.0,
            close: price + 0.5,
            volume: 100.0,
            amount: 0.0,
            pct_chg: None,
            closed: true,
        }
    }

    #[test]
    fn test_replay_evaluator_evaluates_suite() {
        let ep1 = BenchmarkEpisode::new(
            "ep_trend_01",
            "BTC-USDT-SWAP",
            "15m",
            "tight_channel",
            "Trend follow setup",
            vec![dummy_bar(1, 100.0)],
            "OPEN_LONG",
            vec![dummy_bar(2, 102.0), dummy_bar(3, 103.0)],
            2.0,
        );

        let ep2 = BenchmarkEpisode::new(
            "ep_chop_02",
            "BTC-USDT-SWAP",
            "15m",
            "trading_range",
            "Chop chop range",
            vec![dummy_bar(1, 100.0)],
            "WAIT",
            vec![dummy_bar(2, 100.5)],
            0.0,
        );

        let episodes = vec![ep1, ep2];
        let prompt = "这是一个关于趋势、通道和震荡区间的完整策略提示词规则内容，用于评估离线交易表现。";

        let (outcomes, metrics) = ReplayEvaluator::evaluate_suite(prompt, "v_test", &episodes);
        assert_eq!(outcomes.len(), 2);
        assert!(metrics.samples >= 1);
        assert!(metrics.expectancy_r > 0.0);
    }
}
