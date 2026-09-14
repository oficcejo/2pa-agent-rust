//! Durable storage for resolved trade outcomes.
//!
//! One JSON file per `signal_id`, under `records/outcomes/`. The signal id is
//! the receipt that ties a decision, its audit row and its realised result
//! together, so it doubles as the file key.
//!
//! Writes are atomic (temp file + rename) so a crash mid-write cannot leave a
//! truncated outcome behind.

use crate::learning::feedback::TradeOutcome;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct OutcomeStore {
    dir: PathBuf,
}

impl OutcomeStore {
    pub fn new<P: AsRef<Path>>(dir: P) -> Self {
        Self { dir: dir.as_ref().to_path_buf() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_for(&self, signal_id: &str) -> PathBuf {
        // signal ids are hex digests, but sanitise anyway so a malformed id can
        // never escape the outcome directory.
        let safe: String = signal_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        let safe = if safe.is_empty() { "unknown".to_string() } else { safe };
        self.dir.join(format!("{safe}.json"))
    }

    pub fn exists(&self, signal_id: &str) -> bool {
        self.path_for(signal_id).is_file()
    }

    pub fn save(&self, outcome: &TradeOutcome) -> Result<PathBuf> {
        fs::create_dir_all(&self.dir)
            .with_context(|| format!("创建结果目录失败: {}", self.dir.display()))?;
        let path = self.path_for(&outcome.signal_id);
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(outcome)?;
        fs::write(&tmp, text).with_context(|| format!("写入结果文件失败: {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("提交结果文件失败: {}", path.display()))?;
        Ok(path)
    }

    pub fn load(&self, signal_id: &str) -> Option<TradeOutcome> {
        let path = self.path_for(signal_id);
        let text = fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn delete(&self, signal_id: &str) -> bool {
        fs::remove_file(self.path_for(signal_id)).is_ok()
    }

    /// All stored outcomes, newest first by `created_ms`.
    pub fn list(&self, limit: usize) -> Vec<TradeOutcome> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(&self.dir) else { return out };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(o) = serde_json::from_str::<TradeOutcome>(&text) {
                    out.push(o);
                }
            }
        }
        out.sort_by(|a, b| b.created_ms.cmp(&a.created_ms));
        if out.len() > limit {
            out.truncate(limit);
        }
        out
    }

    /// Outcomes that satisfy the qualification policy.
    pub fn list_qualified(&self, limit: usize) -> Vec<TradeOutcome> {
        self.list(limit).into_iter().filter(|o| o.qualified).collect()
    }

    pub fn len(&self) -> usize {
        self.list(usize::MAX).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::feedback::TradeOutcome;

    fn sample(signal_id: &str, qualified: bool, created_ms: i64) -> TradeOutcome {
        TradeOutcome {
            signal_id: signal_id.into(),
            decision_record_id: "d1".into(),
            strategy_id: "2pa_trend".into(),
            strategy_version: "2026-09-v1".into(),
            prompt_version: "v1".into(),
            prompt_hash: "h".into(),
            symbol: "BTC-USDT-SWAP".into(),
            timeframe: "15m".into(),
            side: "long".into(),
            cycle_position: "trending_tr".into(),
            detected_patterns: vec![],
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
            r_multiple: 2.0,
            mfe_r: 2.0,
            mae_r: 0.3,
            hold_bars: 4,
            exit_reason: "take_profit".into(),
            qualified,
            qualification_reason: "ok".into(),
            created_ms,
            resolved_ms: created_ms,
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("okx-outcome-test-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = temp_dir("roundtrip");
        let store = OutcomeStore::new(&dir);
        let o = sample("abc123", true, 1000);
        store.save(&o).unwrap();
        assert!(store.exists("abc123"));
        let loaded = store.load("abc123").unwrap();
        assert_eq!(loaded.signal_id, "abc123");
        assert_eq!(loaded.r_multiple, 2.0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_filters_qualified_and_sorts_newest_first() {
        let dir = temp_dir("list");
        let store = OutcomeStore::new(&dir);
        store.save(&sample("a", true, 100)).unwrap();
        store.save(&sample("b", false, 200)).unwrap();
        store.save(&sample("c", true, 300)).unwrap();

        let all = store.list(10);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].signal_id, "c", "newest first");

        let qualified = store.list_qualified(10);
        assert_eq!(qualified.len(), 2);
        assert!(qualified.iter().all(|o| o.qualified));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_loads_as_none() {
        let dir = temp_dir("missing");
        let store = OutcomeStore::new(&dir);
        assert!(store.load("nope").is_none());
        assert!(!store.exists("nope"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn path_traversal_is_neutralised() {
        let dir = temp_dir("traversal");
        let store = OutcomeStore::new(&dir);
        let mut o = sample("../../etc/passwd", true, 1);
        o.signal_id = "../../etc/passwd".into();
        let path = store.save(&o).unwrap();
        // Must stay inside the outcome directory.
        assert!(path.starts_with(&dir));
        let _ = fs::remove_dir_all(&dir);
    }
}
