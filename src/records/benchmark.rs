use crate::data::base::KlineBar;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A self-contained historical market slice used for offline strategy evaluation.
///
/// Borrowed from Reef's benchmark task set design in Human-Agent-Society:
/// Candidate prompts are evaluated against fixed representative episodes
/// rather than requiring live execution samples, breaking the candidate activation deadlock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkEpisode {
    pub episode_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub market_regime: String,
    pub description: String,
    pub kline_data: Vec<KlineBar>,
    #[serde(default)]
    pub htf_text: Option<String>,
    /// Expected ground-truth action (e.g. "OPEN_LONG", "OPEN_SHORT", "WAIT", "HOLD")
    pub expected_action: String,
    /// Future bars following the analysis point, used to simulate order fills and trade outcomes
    #[serde(default)]
    pub future_bars: Vec<KlineBar>,
    /// Expected benchmark R-multiple if the setup is traded correctly
    #[serde(default)]
    pub benchmark_r: f64,
}

impl BenchmarkEpisode {
    pub fn new(
        episode_id: impl Into<String>,
        symbol: impl Into<String>,
        timeframe: impl Into<String>,
        market_regime: impl Into<String>,
        description: impl Into<String>,
        kline_data: Vec<KlineBar>,
        expected_action: impl Into<String>,
        future_bars: Vec<KlineBar>,
        benchmark_r: f64,
    ) -> Self {
        Self {
            episode_id: episode_id.into(),
            symbol: symbol.into(),
            timeframe: timeframe.into(),
            market_regime: market_regime.into(),
            description: description.into(),
            kline_data,
            htf_text: None,
            expected_action: expected_action.into(),
            future_bars,
            benchmark_r,
        }
    }
}

const DEFAULT_EPISODES: &[&str] = &[
    include_str!("../../records/benchmark_episodes/ep01_btc_spike_long.json"),
    include_str!("../../records/benchmark_episodes/ep02_eth_tight_channel_long.json"),
    include_str!("../../records/benchmark_episodes/ep03_btc_trading_range_wait.json"),
    include_str!("../../records/benchmark_episodes/ep04_eth_range_fade_short.json"),
    include_str!("../../records/benchmark_episodes/ep05_btc_bear_channel_short.json"),
    include_str!("../../records/benchmark_episodes/ep06_sol_wedge_reversal_long.json"),
];

/// Get the compiled-in suite of default benchmark episodes.
pub fn default_benchmark_episodes() -> Vec<BenchmarkEpisode> {
    DEFAULT_EPISODES
        .iter()
        .filter_map(|s| serde_json::from_str::<BenchmarkEpisode>(s).ok())
        .collect()
}

/// Load all benchmark episodes from a directory, falling back to embedded defaults if empty.
pub fn load_benchmark_episodes(dir: &Path) -> Result<Vec<BenchmarkEpisode>> {
    let mut episodes = Vec::new();

    if dir.exists() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    if let Ok(data) = std::fs::read_to_string(&path) {
                        if let Ok(ep) = serde_json::from_str::<BenchmarkEpisode>(&data) {
                            episodes.push(ep);
                        }
                    }
                }
            }
        }
    }

    if episodes.is_empty() {
        episodes = default_benchmark_episodes();
        // If directory exists or can be created, seed them to disk
        if !episodes.is_empty() && (dir.exists() || std::fs::create_dir_all(dir).is_ok()) {
            for ep in &episodes {
                let _ = save_benchmark_episode(dir, ep);
            }
        }
    }

    episodes.sort_by(|a, b| a.episode_id.cmp(&b.episode_id));
    Ok(episodes)
}

/// Save a benchmark episode to the given directory.
pub fn save_benchmark_episode(dir: &Path, episode: &BenchmarkEpisode) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("创建基准切片目录失败: {}", dir.display()))?;

    let filename = format!("{}_{}_{}.json", episode.episode_id, episode.symbol, episode.market_regime);
    let target = dir.join(filename);
    let json = serde_json::to_string_pretty(episode)?;
    std::fs::write(&target, json).with_context(|| format!("写入基准切片失败: {}", target.display()))?;

    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_benchmark_episode_serde_roundtrip() {
        let ep = BenchmarkEpisode::new(
            "ep_01",
            "BTC-USDT-SWAP",
            "15m",
            "tight_channel",
            "Test episode",
            vec![],
            "OPEN_LONG",
            vec![],
            2.5,
        );

        let json = serde_json::to_string(&ep).unwrap();
        let deserialized: BenchmarkEpisode = serde_json::from_str(&json).unwrap();
        assert_eq!(ep, deserialized);
    }
}
