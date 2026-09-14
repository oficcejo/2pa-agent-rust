//! Turns resolved outcomes into experience-library cases.
//!
//! The experience library already existed and was already read by
//! `ExperienceReader`, but nothing ever wrote to it: the case files were
//! maintained by hand. This module closes that loop. Only outcomes that pass
//! the qualification policy are recorded, and each case is written at most
//! once (keyed by `signal_id`) so re-running reconciliation is idempotent.
//!
//! File naming follows the convention `ExperienceReader` parses:
//! `<YYYY-MM-DD_HH-MM-SS>_<signal_id>.json`.

use crate::learning::feedback::{ExitReason, TradeOutcome};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ExperienceWriter {
    dir: PathBuf,
}

impl ExperienceWriter {
    pub fn new<P: AsRef<Path>>(dir: P) -> Self {
        Self { dir: dir.as_ref().to_path_buf() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn case_dir(&self, cycle_position: &str) -> PathBuf {
        let cycle = if cycle_position.trim().is_empty() {
            "unknown".to_string()
        } else {
            sanitise(cycle_position)
        };
        self.dir.join(cycle)
    }

    fn is_success(outcome: &TradeOutcome) -> bool {
        outcome.r_multiple > 0.0
    }

    fn case_filename(outcome: &TradeOutcome) -> String {
        let ts = outcome.resolved_ms.max(outcome.created_ms);
        let stamp = DateTime::from_timestamp_millis(ts)
            .unwrap_or_else(Utc::now)
            .format("%Y-%m-%d_%H-%M-%S")
            .to_string();
        format!("{}_{}.json", stamp, sanitise(&outcome.signal_id))
    }

    /// Whether a case for this signal has already been recorded.
    pub fn has_case(&self, signal_id: &str) -> bool {
        let needle = sanitise(signal_id);
        for cycle in list_cycles(&self.dir) {
            for kind in ["success_cases", "failure_cases"] {
                let dir = self.dir.join(&cycle).join(kind);
                let Ok(entries) = fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    if entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(&format!("_{needle}.json"))
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Record one outcome. Returns the written path, or `None` when the sample
    /// is unqualified or already present.
    pub fn record(&self, outcome: &TradeOutcome) -> Result<Option<PathBuf>> {
        if !outcome.qualified {
            return Ok(None);
        }
        if self.has_case(&outcome.signal_id) {
            return Ok(None);
        }

        let kind = if Self::is_success(outcome) { "success_cases" } else { "failure_cases" };
        let dir = self.case_dir(&outcome.cycle_position).join(kind);
        fs::create_dir_all(&dir)
            .with_context(|| format!("创建经验库目录失败: {}", dir.display()))?;

        let path = dir.join(Self::case_filename(outcome));
        let text = serde_json::to_string_pretty(&render_case(outcome))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &path)?;
        Ok(Some(path))
    }

    /// Record every qualified outcome in a batch. Returns how many were newly
    /// written.
    pub fn sync(&self, outcomes: &[TradeOutcome]) -> Result<usize> {
        let mut written = 0;
        for outcome in outcomes {
            if self.record(outcome)?.is_some() {
                written += 1;
            }
        }
        Ok(written)
    }

    /// Counters for the Web console.
    pub fn stats(&self) -> Value {
        let mut cycles = Vec::new();
        for cycle in list_cycles(&self.dir) {
            let count = |kind: &str| {
                fs::read_dir(self.dir.join(&cycle).join(kind))
                    .map(|entries| {
                        entries
                            .flatten()
                            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
                            .count()
                    })
                    .unwrap_or(0)
            };
            let success = count("success_cases");
            let failure = count("failure_cases");
            if success + failure > 0 {
                cycles.push(json!({
                    "cycle_position": cycle,
                    "success_cases": success,
                    "failure_cases": failure,
                }));
            }
        }
        json!({ "cycles": cycles })
    }
}

fn render_case(outcome: &TradeOutcome) -> Value {
    let is_success = ExperienceWriter::is_success(outcome);
    let exit = ExitReason::parse(&outcome.exit_reason)
        .map(|e| e.as_str().to_string())
        .unwrap_or_else(|| outcome.exit_reason.clone());

    json!({
        "signal_id": outcome.signal_id,
        "case_type": if is_success { "success" } else { "failure" },
        "trading_system": outcome.strategy_id,
        "strategy_version": outcome.strategy_version,
        "prompt_version": outcome.prompt_version,
        "prompt_hash": outcome.prompt_hash,
        "symbol": outcome.symbol,
        "timeframe": outcome.timeframe,
        "cycle_position": outcome.cycle_position,
        "direction": outcome.side,
        "detected_patterns": outcome.detected_patterns,
        "entry_price": outcome.entry_price,
        "stop_loss_price": outcome.stop_price,
        "take_profit_price": outcome.target_price,
        "exit_price": outcome.exit_price,
        "outcome": {
            "r_multiple": round(outcome.r_multiple),
            "mfe_r": round(outcome.mfe_r),
            "mae_r": round(outcome.mae_r),
            "hold_bars": outcome.hold_bars,
            "exit_reason": exit,
            "fees_usd": round(outcome.fees_usd),
            "realized_pnl_usd": round(outcome.realized_pnl_usd),
            "filled": outcome.filled,
        },
        "lesson": render_lesson(outcome, is_success, &exit),
    })
}

fn render_lesson(outcome: &TradeOutcome, is_success: bool, exit: &str) -> String {
    let side_zh = if outcome.side == "long" { "做多" } else { "做空" };
    if is_success {
        format!(
            "同类结构（{} / {}）在 {} 上按计划兑现，出场原因 {}，实现 {:.2}R，过程最大浮盈 {:.2}R、最大回撤 {:.2}R，持仓 {} 根 K 线。\
             说明该结构与止损设置在本例中有效，可复现同类入场条件。",
            outcome.cycle_position,
            outcome.detected_patterns.join(","),
            side_zh,
            exit,
            outcome.r_multiple,
            outcome.mfe_r,
            outcome.mae_r,
            outcome.hold_bars
        )
    } else {
        format!(
            "同类结构（{} / {}）在 {} 上未按计划兑现，出场原因 {}，实现 {:.2}R（禁止记为分批止盈），过程最大浮盈 {:.2}R、最大回撤 {:.2}R，持仓 {} 根 K 线。\
             复盘时须检查：入场确认是否不足、止损是否被微观噪音扫掉、是否存在逆高周期趋势的对抗交易。",
            outcome.cycle_position,
            outcome.detected_patterns.join(","),
            side_zh,
            exit,
            outcome.r_multiple,
            outcome.mfe_r,
            outcome.mae_r,
            outcome.hold_bars
        )
    }
}

fn round(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

fn sanitise(raw: &str) -> String {
    let safe: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if safe.is_empty() { "unknown".to_string() } else { safe }
}

/// Directories under the experience root, skipping any stray files.
pub fn list_cycles(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else { return Vec::new() };
    let mut cycles: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .collect();
    cycles.sort();
    cycles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::feedback::TradeOutcome;

    fn sample(signal_id: &str, r: f64, cycle: &str) -> TradeOutcome {
        TradeOutcome {
            signal_id: signal_id.into(),
            decision_record_id: "rec-1".into(),
            strategy_id: "2pa_trend".into(),
            strategy_version: "2026-09-v1".into(),
            prompt_version: "v1".into(),
            prompt_hash: "hash".into(),
            symbol: "BTC-USDT-SWAP".into(),
            timeframe: "15m".into(),
            side: "long".into(),
            cycle_position: cycle.into(),
            detected_patterns: vec!["h2".into(), "breakout_test".into()],
            entry_price: 100.0,
            stop_price: 99.0,
            target_price: 102.0,
            exit_price: Some(102.0),
            size: 1.0,
            filled: true,
            fill_ratio: 1.0,
            fees_usd: 0.1,
            realized_pnl_usd: 1.9,
            pnl_source: "model_estimate".into(),
            r_multiple: r,
            mfe_r: 2.0,
            mae_r: 0.2,
            hold_bars: 5,
            exit_reason: "take_profit".into(),
            qualified: true,
            qualification_reason: "ok".into(),
            created_ms: 1_700_000_000_000,
            resolved_ms: 1_700_000_000_000,
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("okx-exp-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_success_and_failure_into_correct_buckets() {
        let dir = temp_dir("buckets");
        let w = ExperienceWriter::new(&dir);
        let win = w.record(&sample("win1", 2.0, "trending_tr")).unwrap().unwrap();
        assert!(win.to_string_lossy().contains("success_cases"));
        let loss = w.record(&sample("loss1", -1.0, "trending_tr")).unwrap().unwrap();
        assert!(loss.to_string_lossy().contains("failure_cases"));
        assert!(win.file_name().unwrap().to_string_lossy().ends_with("_win1.json"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn case_matches_experience_reader_filename_convention() {
        let dir = temp_dir("naming");
        let w = ExperienceWriter::new(&dir);
        let path = w.record(&sample("abc", 1.0, "spike")).unwrap().unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let re = regex::Regex::new(r"^(\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2})_abc\.json$").unwrap();
        assert!(re.is_match(&name), "unexpected filename: {name}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn record_is_idempotent() {
        let dir = temp_dir("idempotent");
        let w = ExperienceWriter::new(&dir);
        assert!(w.record(&sample("dup", 1.0, "spike")).unwrap().is_some());
        assert!(w.record(&sample("dup", 1.0, "spike")).unwrap().is_none());
        assert!(w.has_case("dup"));
        let stats = w.stats();
        assert_eq!(stats["cycles"][0]["success_cases"], 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unqualified_samples_are_not_recorded() {
        let dir = temp_dir("unqualified");
        let w = ExperienceWriter::new(&dir);
        let mut o = sample("bad", 1.0, "spike");
        o.qualified = false;
        assert!(w.record(&o).unwrap().is_none());
        assert!(!w.has_case("bad"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn case_carries_fields_the_reader_scores_on() {
        let dir = temp_dir("reader");
        let w = ExperienceWriter::new(&dir);
        let path = w.record(&sample("scored", 2.0, "trending_tr")).unwrap().unwrap();
        let val: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(val["direction"], "long");
        assert_eq!(val["detected_patterns"][0], "h2");
        assert_eq!(val["outcome"]["r_multiple"], 2.0);
        assert!(val["lesson"].as_str().unwrap().contains("做多"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_counts_only_new_cases() {
        let dir = temp_dir("sync");
        let w = ExperienceWriter::new(&dir);
        let batch = vec![sample("a", 1.0, "spike"), sample("b", -1.0, "spike")];
        assert_eq!(w.sync(&batch).unwrap(), 2);
        assert_eq!(w.sync(&batch).unwrap(), 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
