//! Persistent state-fingerprint decision cache.
//! Stores and reuses LLM Stage 1/2 decisions across backtest runs with zero token cost for reruns.

use crate::data::base::KlineFrame;
use anyhow::{Context, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Cached decision produced by LLM or deterministic fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedDecision {
    pub strategy_id: String,
    pub action: String,
    pub order_type: String,
    pub order_direction: Option<String>,
    pub entry_price: Option<f64>,
    pub stop_loss_price: Option<f64>,
    pub take_profit_price: Option<f64>,
    pub trade_confidence: u32,
    pub reasoning: String,
    pub stage1_diagnosis: Option<serde_json::Value>,
    pub raw_decision: serde_json::Value,
    pub timestamp_ms: i64,
}

/// Persistent cache holding decision fingerprints.
#[derive(Debug, Clone)]
pub struct DecisionCache {
    path: PathBuf,
    entries: Arc<RwLock<HashMap<String, CachedDecision>>>,
    dirty: Arc<RwLock<bool>>,
}

impl DecisionCache {
    /// Initialize cache from default or given path.
    pub fn new(cache_path: Option<PathBuf>) -> Self {
        let path = cache_path.unwrap_or_else(|| {
            PathBuf::from("records").join("backtest_decision_cache.json")
        });

        let mut entries = HashMap::new();
        if path.exists() {
            if let Ok(file) = File::open(&path) {
                let reader = BufReader::new(file);
                if let Ok(loaded) = serde_json::from_reader::<_, HashMap<String, CachedDecision>>(reader) {
                    info!("Loaded {} cached decisions from {:?}", loaded.len(), path);
                    entries = loaded;
                } else {
                    warn!("Failed to parse decision cache at {:?}, starting fresh", path);
                }
            }
        }

        Self {
            path,
            entries: Arc::new(RwLock::new(entries)),
            dirty: Arc::new(RwLock::new(false)),
        }
    }

    /// Compute a unique cryptographic fingerprint for a KlineFrame state and strategy.
    pub fn compute_fingerprint(
        strategy_id: &str,
        frame: &KlineFrame,
        htf_frame: Option<&KlineFrame>,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(strategy_id.as_bytes());
        hasher.update(frame.symbol.as_bytes());
        hasher.update(frame.timeframe.as_bytes());

        if let Some(b0) = frame.bars.first() {
            hasher.update(b0.ts_open.to_le_bytes());
            hasher.update(b0.open.to_bits().to_le_bytes());
            hasher.update(b0.high.to_bits().to_le_bytes());
            hasher.update(b0.low.to_bits().to_le_bytes());
            hasher.update(b0.close.to_bits().to_le_bytes());
            hasher.update(b0.volume.to_bits().to_le_bytes());
        }

        // Include key indicators to distinguish identical price with different momentum
        if let Some(&ema) = frame.indicators.ema20.first() {
            hasher.update(ema.to_bits().to_le_bytes());
        }
        if let Some(&atr) = frame.indicators.atr14.first() {
            hasher.update(atr.to_bits().to_le_bytes());
        }
        if let Some(&sma) = frame.indicators.sma14.first() {
            hasher.update(sma.to_bits().to_le_bytes());
        }
        if let Some(&s170) = frame.indicators.sma170.first() {
            hasher.update(s170.to_bits().to_le_bytes());
        }

        // Include HTF state if present
        if let Some(h) = htf_frame {
            hasher.update(b"HTF");
            if let Some(hb0) = h.bars.first() {
                hasher.update(hb0.ts_open.to_le_bytes());
                hasher.update(hb0.close.to_bits().to_le_bytes());
            }
        }

        hex::encode(hasher.finalize())
    }

    /// Lookup a decision by its fingerprint key.
    pub fn get(&self, fingerprint: &str) -> Option<CachedDecision> {
        self.entries.read().get(fingerprint).cloned()
    }

    /// Insert a decision into cache and mark dirty.
    pub fn insert(&self, fingerprint: String, decision: CachedDecision) {
        let mut map = self.entries.write();
        map.insert(fingerprint, decision);
        *self.dirty.write() = true;
    }

    /// Count total cached decisions.
    pub fn len(&self) -> usize {
        self.entries.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.read().is_empty()
    }

    /// Save cache to disk if modified.
    pub fn save(&self) -> Result<()> {
        if !*self.dirty.read() {
            return Ok(());
        }

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {:?}", parent))?;
        }

        let file = File::create(&self.path)
            .with_context(|| format!("Failed to create cache file {:?}", self.path))?;
        let writer = BufWriter::new(file);

        let map = self.entries.read();
        serde_json::to_writer_pretty(writer, &*map)
            .with_context(|| format!("Failed to write cache to {:?}", self.path))?;

        *self.dirty.write() = false;
        debug!("Persisted {} decision cache entries to {:?}", map.len(), self.path);
        Ok(())
    }
}
