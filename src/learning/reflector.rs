use crate::learning::feedback::TradeOutcome;
use serde::{Deserialize, Serialize};

/// Structured attribution of a trade failure, diagnosing the root cause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureAttribution {
    pub signal_id: String,
    pub symbol: String,
    pub strategy_id: String,
    pub failure_mode: String,
    pub severity: f64,
    pub root_cause: String,
    pub suggested_rule: String,
}

/// Analyzes losing trade trajectories to attribute failure modes.
pub struct StrategyReflector;

impl StrategyReflector {
    /// Attribute a single outcome if it represents a failure.
    pub fn attribute_failure(outcome: &TradeOutcome) -> Option<FailureAttribution> {
        if outcome.r_multiple >= 0.0 {
            return None;
        }

        let severity = (-outcome.r_multiple).max(0.5);
        let patterns = &outcome.detected_patterns;
        let regime = outcome.cycle_position.to_lowercase();
        let hold_bars = outcome.hold_bars;

        let (failure_mode, root_cause, suggested_rule) = if hold_bars <= 2 {
            (
                "whipsaw_tight_stop".to_string(),
                format!("持仓仅 {} 根 K 线即被止损出局，止损空间过窄或遭遇噪音打损", hold_bars),
                "结构性止损点必须预留至少 0.3~0.5 ATR 缓冲区，禁止将止损紧贴密集波段极值点。".to_string(),
            )
        } else if regime.contains("range") || regime.contains("chop") || regime.contains("barbwire") {
            (
                "range_breakout_trap".to_string(),
                "在震荡区间 (Trading Range) 或铁丝网内部追单，遭遇假突破反向收割".to_string(),
                "震荡区间中轴区域严禁顺势开仓；接近区间边界时仅允许顺大势高抛低吸，未确认突破前一律观望 (WAIT)。".to_string(),
            )
        } else if patterns.iter().any(|p| p.contains("reversal") || p.contains("bottom") || p.contains("top")) {
            (
                "counter_trend_knife_catching".to_string(),
                "强单边趋势中过早尝试摸顶抄底，逆势接飞刀".to_string(),
                "强趋势急速 (Spike) 运行期间严禁逆势第一波介入，必须等待两次尝试衰竭 (H2/L2) 或大级别 MTR 结构出现。".to_string(),
            )
        } else if outcome.exit_reason == "closed_early" {
            (
                "premature_discretionary_exit".to_string(),
                "持仓过程中过早主动平仓，导致计划期望 R 无法兑现".to_string(),
                "除非原开仓结构彻底失效或出现重大相反信号，否则严格按照 SL/TP 机械执行，避免恐慌提前退出。".to_string(),
            )
        } else {
            (
                "structural_invalidation".to_string(),
                format!("行情突破技术无效点，止损出局 (R: {:.2})", outcome.r_multiple),
                "进场前必须核验更高时间框架 (HTF) 关键阻力与支撑，顺大势方向开仓。".to_string(),
            )
        };

        Some(FailureAttribution {
            signal_id: outcome.signal_id.clone(),
            symbol: outcome.symbol.clone(),
            strategy_id: outcome.strategy_id.clone(),
            failure_mode,
            severity,
            root_cause,
            suggested_rule,
        })
    }

    /// Attribute all failures in an outcome list.
    pub fn attribute_failures(outcomes: &[TradeOutcome]) -> Vec<FailureAttribution> {
        outcomes.iter().filter_map(Self::attribute_failure).collect()
    }
}

/// Candidate prompt mutation proposal generated from failure attributions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub proposal_id: String,
    pub base_version: String,
    pub proposed_version: String,
    pub diff_summary: String,
    pub mutated_content: String,
    pub addressed_modes: Vec<String>,
}

/// Proposes targeted local prompt mutations (GEPA-style local diffs) based on failure reflections.
pub struct Proposer;

impl Proposer {
    /// Generate a targeted prompt mutation from failure attributions.
    pub fn propose_mutation(
        incumbent_content: &str,
        incumbent_version: &str,
        attributions: &[FailureAttribution],
    ) -> Option<Proposal> {
        if attributions.is_empty() {
            return None;
        }

        let proposal_id = format!("prop_{}", uuid::Uuid::new_v4().simple());
        let proposed_version = format!("{}_gepa_{}", incumbent_version, &proposal_id[..6]);

        let mut diff_rules = Vec::new();
        let mut addressed_modes = Vec::new();

        for attr in attributions {
            if !addressed_modes.contains(&attr.failure_mode) {
                addressed_modes.push(attr.failure_mode.clone());
                diff_rules.push(format!(
                    "### 【GEPA 负反馈防御条款 - {}】\n- **失效归因**: {}\n- **强制约束**: {}\n",
                    attr.failure_mode, attr.root_cause, attr.suggested_rule
                ));
            }
        }

        let diff_patch = format!(
            "\n\n## 🔄 【GEPA 反思突变补丁 - {}】\n{}\n",
            proposal_id,
            diff_rules.join("\n")
        );

        let mutated_content = format!("{}{}", incumbent_content.trim_end(), diff_patch);
        let diff_summary = format!(
            "基于 {} 项失败归因分析，注入 {} 条局部防御条款：{}",
            attributions.len(),
            addressed_modes.len(),
            addressed_modes.join(", ")
        );

        Some(Proposal {
            proposal_id,
            base_version: incumbent_version.to_string(),
            proposed_version,
            diff_summary,
            mutated_content,
            addressed_modes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_loss(mode: &str, r: f64, hold_bars: u32) -> TradeOutcome {
        TradeOutcome {
            signal_id: "sig_loss_1".to_string(),
            decision_record_id: "rec_1".to_string(),
            strategy_id: "2pa_trend".to_string(),
            strategy_version: "v1".to_string(),
            prompt_version: "v1".to_string(),
            prompt_hash: "hash".to_string(),
            symbol: "BTC-USDT-SWAP".to_string(),
            timeframe: "15m".to_string(),
            side: "long".to_string(),
            cycle_position: if mode == "range" { "trading_range".to_string() } else { "spike".to_string() },
            detected_patterns: if mode == "reversal" { vec!["wedge_bottom".to_string()] } else { vec![] },
            entry_price: 100.0,
            stop_price: 99.0,
            target_price: 103.0,
            exit_price: Some(99.0),
            size: 1.0,
            filled: true,
            fill_ratio: 1.0,
            fees_usd: 1.0,
            realized_pnl_usd: -100.0,
            pnl_source: "model".to_string(),
            r_multiple: r,
            mfe_r: 0.1,
            mae_r: 1.0,
            hold_bars,
            exit_reason: "stop_loss".to_string(),
            qualified: true,
            qualification_reason: "合格样本".to_string(),
            created_ms: 1000,
            resolved_ms: 2000,
        }
    }

    #[test]
    fn test_reflector_attributes_whipsaw_and_range() {
        let loss1 = dummy_loss("other", -1.0, 1);
        let attr1 = StrategyReflector::attribute_failure(&loss1).expect("attribute loss1");
        assert_eq!(attr1.failure_mode, "whipsaw_tight_stop");

        let loss2 = dummy_loss("range", -1.0, 10);
        let attr2 = StrategyReflector::attribute_failure(&loss2).expect("attribute loss2");
        assert_eq!(attr2.failure_mode, "range_breakout_trap");
    }

    #[test]
    fn test_proposer_generates_local_diff_mutation() {
        let loss = dummy_loss("range", -1.0, 5);
        let attrs = StrategyReflector::attribute_failures(&[loss]);
        let base_prompt = "基础策略规则文本。";

        let proposal = Proposer::propose_mutation(base_prompt, "v1", &attrs).expect("proposal");
        assert!(proposal.mutated_content.starts_with(base_prompt));
        assert!(proposal.mutated_content.contains("GEPA 反思突变补丁"));
        assert!(proposal.mutated_content.contains("range_breakout_trap"));
        assert!(proposal.mutated_content.contains("震荡区间中轴区域严禁顺势开仓"));
        assert!(proposal.diff_summary.contains("range_breakout_trap"));
    }
}
