//! Candidate evaluation and selective publication.
//!
//! Reef only commits an update after a candidate beats the incumbent on a
//! representative task set; nothing is promoted automatically. The trading
//! analogue is stricter, because the cost function is money: a prompt revision
//! is published as an inactive artifact, then compared against the incumbent
//! on recorded results. Only a candidate that does not degrade the incumbent
//! is eligible for activation, and activation always stays a separate,
//! deliberate step.
//!
//! The comparison runs on realised R multiples, not on model-reported
//! confidence, which the project already documents as explicitly not a win
//! rate.

use crate::learning::feedback::TradeOutcome;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Aggregate performance of a set of outcomes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StrategyMetrics {
    pub samples: usize,
    pub wins: usize,
    pub losses: usize,
    pub win_rate: f64,
    /// Mean realised R multiple. The headline number.
    pub expectancy_r: f64,
    pub avg_mfe_r: f64,
    pub avg_mae_r: f64,
    pub avg_hold_bars: f64,
    /// Gross profit divided by gross loss; `None` when there are no losses.
    pub profit_factor: Option<f64>,
    pub total_pnl_usd: f64,
    pub total_fees_usd: f64,
}

/// Summarise outcomes. Only filled, qualified samples are counted, so macro
/// decisions and cancelled entries cannot flatter the numbers.
pub fn metrics_for(outcomes: &[TradeOutcome]) -> StrategyMetrics {
    let sample: Vec<&TradeOutcome> = outcomes
        .iter()
        .filter(|o| o.filled && o.qualified && o.r_multiple.is_finite())
        .collect();

    let n = sample.len();
    if n == 0 {
        return StrategyMetrics::default();
    }

    let wins = sample.iter().filter(|o| o.r_multiple > 0.0).count();
    let losses = n - wins;
    let sum_r: f64 = sample.iter().map(|o| o.r_multiple).sum();
    let gross_profit: f64 = sample.iter().filter(|o| o.r_multiple > 0.0).map(|o| o.r_multiple).sum();
    let gross_loss: f64 = sample.iter().filter(|o| o.r_multiple < 0.0).map(|o| -o.r_multiple).sum();

    StrategyMetrics {
        samples: n,
        wins,
        losses,
        win_rate: wins as f64 / n as f64,
        expectancy_r: sum_r / n as f64,
        avg_mfe_r: sample.iter().map(|o| o.mfe_r).sum::<f64>() / n as f64,
        avg_mae_r: sample.iter().map(|o| o.mae_r).sum::<f64>() / n as f64,
        avg_hold_bars: sample.iter().map(|o| o.hold_bars as f64).sum::<f64>() / n as f64,
        profit_factor: if gross_loss > 0.0 { Some(gross_profit / gross_loss) } else { None },
        total_pnl_usd: sample.iter().map(|o| o.realized_pnl_usd).sum(),
        total_fees_usd: sample.iter().map(|o| o.fees_usd).sum(),
    }
}

/// Group outcomes by the prompt version that produced them.
pub fn group_by_prompt_version(outcomes: &[TradeOutcome]) -> BTreeMap<String, Vec<TradeOutcome>> {
    let mut grouped: BTreeMap<String, Vec<TradeOutcome>> = BTreeMap::new();
    for o in outcomes {
        let key = if o.prompt_version.trim().is_empty() {
            "unknown".to_string()
        } else {
            o.prompt_version.clone()
        };
        grouped.entry(key).or_default().push(o.clone());
    }
    grouped
}

/// Rules a candidate must satisfy before it may be activated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationPolicy {
    /// Minimum qualified samples on the candidate before any verdict is made.
    pub min_samples: usize,
    /// The candidate's expectancy must exceed the baseline by at least this
    /// much (in R). `0.0` means "must not be worse".
    pub min_expectancy_delta_r: f64,
    /// Tolerated drop in win rate versus the baseline.
    pub max_win_rate_drop: f64,
}

impl Default for EvaluationPolicy {
    fn default() -> Self {
        Self { min_samples: 20, min_expectancy_delta_r: 0.0, max_win_rate_drop: 0.05 }
    }
}

/// Outcome of a candidate comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub accepted: bool,
    pub reasons: Vec<String>,
    pub candidate: StrategyMetrics,
    pub baseline: StrategyMetrics,
    /// Difference in expectancy (candidate − baseline), in R.
    pub expectancy_delta_r: f64,
}

impl Verdict {
    pub fn summary(&self) -> String {
        if self.accepted {
            "候选版本不劣于当前版本，可以进入人工发布确认".to_string()
        } else {
            format!("候选版本未通过选择性发布：{}", self.reasons.join("；"))
        }
    }
}

/// Compare a candidate prompt version against the incumbent.
///
/// The baseline is the incumbent's metrics; the candidate is the new version's.
/// With too few samples the result is an explicit rejection rather than an
/// optimistic default, matching the project's "do not claim what has not been
/// measured" stance.
pub fn compare_candidate(
    candidate: &StrategyMetrics,
    baseline: &StrategyMetrics,
    policy: &EvaluationPolicy,
) -> Verdict {
    let mut reasons = Vec::new();

    if candidate.samples < policy.min_samples {
        reasons.push(format!(
            "候选样本不足：{} / {}，无法进行统计比较",
            candidate.samples, policy.min_samples
        ));
    }
    if baseline.samples < policy.min_samples {
        reasons.push(format!(
            "基线样本不足：{} / {}，尚无可比较的基准",
            baseline.samples, policy.min_samples
        ));
    }

    let delta = candidate.expectancy_r - baseline.expectancy_r;
    if candidate.samples >= policy.min_samples
        && baseline.samples >= policy.min_samples
        && delta < policy.min_expectancy_delta_r
    {
        reasons.push(format!(
            "期望 R 未达门槛：候选 {:.3}R，基线 {:.3}R，差值 {:.3}R 低于要求 {:.3}R",
            candidate.expectancy_r, baseline.expectancy_r, delta, policy.min_expectancy_delta_r
        ));
    }

    let win_rate_drop = baseline.win_rate - candidate.win_rate;
    if candidate.samples >= policy.min_samples
        && baseline.samples >= policy.min_samples
        && win_rate_drop > policy.max_win_rate_drop
    {
        reasons.push(format!(
            "胜率下降 {:.1} 个百分点，超过容忍度 {:.1} 个百分点",
            win_rate_drop * 100.0,
            policy.max_win_rate_drop * 100.0
        ));
    }

    Verdict {
        accepted: reasons.is_empty(),
        reasons,
        candidate: candidate.clone(),
        baseline: baseline.clone(),
        expectancy_delta_r: delta,
    }
}

/// Evaluate every published prompt version against the currently active one.
pub fn evaluate_versions(
    outcomes: &[TradeOutcome],
    active_version: &str,
    policy: &EvaluationPolicy,
) -> BTreeMap<String, Verdict> {
    let grouped = group_by_prompt_version(outcomes);
    let baseline = metrics_for(grouped.get(active_version).map(|v| v.as_slice()).unwrap_or(&[]));

    grouped
        .iter()
        .filter(|(version, _)| version.as_str() != active_version)
        .map(|(version, samples)| {
            (version.clone(), compare_candidate(&metrics_for(samples), &baseline, policy))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::feedback::TradeOutcome;

    fn outcome(r: f64, prompt_version: &str, filled: bool, qualified: bool) -> TradeOutcome {
        TradeOutcome {
            signal_id: format!("s-{prompt_version}-{r}"),
            decision_record_id: "d".into(),
            strategy_id: "2pa_trend".into(),
            strategy_version: "2026-09-v1".into(),
            prompt_version: prompt_version.into(),
            prompt_hash: "h".into(),
            symbol: "BTC-USDT-SWAP".into(),
            timeframe: "15m".into(),
            side: "long".into(),
            cycle_position: "trending_tr".into(),
            detected_patterns: vec![],
            entry_price: 100.0,
            stop_price: 99.0,
            target_price: 104.0,
            exit_price: Some(100.0 + r),
            size: 1.0,
            filled,
            fill_ratio: 1.0,
            fees_usd: 0.1,
            realized_pnl_usd: r,
            pnl_source: "model_estimate".into(),
            r_multiple: r,
            mfe_r: r.max(0.0),
            mae_r: (-r).max(0.0),
            hold_bars: 4,
            exit_reason: if r > 0.0 { "take_profit".into() } else { "stop_loss".into() },
            qualified,
            qualification_reason: "ok".into(),
            created_ms: 0,
            resolved_ms: 0,
        }
    }

    fn series(rs: &[f64], version: &str) -> Vec<TradeOutcome> {
        rs.iter().map(|r| outcome(*r, version, true, true)).collect()
    }

    #[test]
    fn metrics_ignore_unfilled_and_unqualified() {
        let mut outs = series(&[2.0, -1.0], "v1");
        outs.push(outcome(5.0, "v1", false, true)); // unfilled
        outs.push(outcome(9.0, "v1", true, false)); // unqualified
        let m = metrics_for(&outs);
        assert_eq!(m.samples, 2);
        assert_eq!(m.wins, 1);
        assert_eq!(m.losses, 1);
        assert_eq!(m.expectancy_r, 0.5);
        assert_eq!(m.win_rate, 0.5);
    }

    #[test]
    fn empty_metrics_are_zero() {
        let m = metrics_for(&[]);
        assert_eq!(m.samples, 0);
        assert_eq!(m.expectancy_r, 0.0);
        assert!(m.profit_factor.is_none());
    }

    #[test]
    fn profit_factor_is_none_without_losses() {
        let m = metrics_for(&series(&[1.0, 2.0], "v1"));
        assert!(m.profit_factor.is_none());
        let m = metrics_for(&series(&[2.0, -1.0], "v1"));
        assert_eq!(m.profit_factor, Some(2.0));
    }

    #[test]
    fn candidate_is_rejected_when_samples_are_thin() {
        let policy = EvaluationPolicy { min_samples: 20, ..Default::default() };
        let candidate = metrics_for(&series(&[1.0, 1.0], "v2"));
        let baseline = metrics_for(&series(&[0.5; 30], "v1"));
        let v = compare_candidate(&candidate, &baseline, &policy);
        assert!(!v.accepted);
        assert!(v.reasons.iter().any(|r| r.contains("候选样本不足")));
    }

    #[test]
    fn candidate_is_rejected_when_expectancy_degrades() {
        let policy = EvaluationPolicy { min_samples: 3, min_expectancy_delta_r: 0.0, max_win_rate_drop: 0.05 };
        let baseline = metrics_for(&series(&[1.0, 1.0, 1.0], "v1"));
        let candidate = metrics_for(&series(&[0.2, 0.2, 0.2], "v2"));
        let v = compare_candidate(&candidate, &baseline, &policy);
        assert!(!v.accepted);
        assert!(v.reasons.iter().any(|r| r.contains("期望 R 未达门槛")));
        assert!(v.expectancy_delta_r < 0.0);
    }

    #[test]
    fn candidate_is_accepted_when_it_does_not_degrade_incumbent() {
        let policy = EvaluationPolicy { min_samples: 3, min_expectancy_delta_r: 0.0, max_win_rate_drop: 0.05 };
        let baseline = metrics_for(&series(&[0.4, 0.4, 0.4], "v1"));
        let candidate = metrics_for(&series(&[0.9, 0.6, 0.5], "v2"));
        let v = compare_candidate(&candidate, &baseline, &policy);
        assert!(v.accepted, "{:?}", v.reasons);
        assert!(v.expectancy_delta_r > 0.0);
        assert!(v.summary().contains("人工发布确认"));
    }

    #[test]
    fn evaluate_versions_uses_active_as_baseline() {
        let mut outs = series(&[1.0, 1.0, 1.0], "v1");
        outs.extend(series(&[0.1, 0.1, 0.1], "v2"));
        let policy = EvaluationPolicy { min_samples: 3, ..Default::default() };
        let verdicts = evaluate_versions(&outs, "v1", &policy);
        assert_eq!(verdicts.len(), 1);
        assert!(!verdicts["v2"].accepted);
        assert_eq!(verdicts["v2"].baseline.samples, 3);
    }

    #[test]
    fn grouping_labels_empty_versions_as_unknown() {
        let outs = vec![outcome(1.0, "", true, true)];
        let grouped = group_by_prompt_version(&outs);
        assert!(grouped.contains_key("unknown"));
    }
}
