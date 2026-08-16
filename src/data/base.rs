use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KlineBar {
    pub seq: usize,        // 1 = newest closed bar, N = oldest; 0 = forming bar
    pub ts_open: i64,      // Unix timestamp in milliseconds (UTC) of bar open
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    #[serde(default)]
    pub amount: f64,
    #[serde(default)]
    pub pct_chg: Option<f64>,
    #[serde(default = "default_true")]
    pub closed: bool,
}

fn default_true() -> bool { true }

impl KlineBar {
    pub fn normalized(&self) -> Self {
        let high = self.high.max(self.low);
        let low = self.high.min(self.low);
        let close = self.close.max(low).min(high);
        KlineBar {
            seq: self.seq,
            ts_open: self.ts_open,
            open: self.open,
            high,
            low,
            close,
            volume: self.volume,
            amount: self.amount,
            pct_chg: self.pct_chg,
            closed: self.closed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndicatorBundle {
    pub ema20: Vec<f64>,
    pub atr14: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KlineFrame {
    pub symbol: String,
    pub timeframe: String,
    pub bars: Vec<KlineBar>, // bars[0] is newest, bars[last] is oldest
    pub indicators: IndicatorBundle,
    pub snapshot_ts_local_ms: i64,
}
