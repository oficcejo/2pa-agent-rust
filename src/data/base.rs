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
    #[serde(default)]
    pub sma14: Vec<f64>,
    #[serde(default)]
    pub sma170: Vec<f64>,
    #[serde(default)]
    pub dev170_pct: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KlineFrame {
    pub symbol: String,
    pub timeframe: String,
    pub bars: Vec<KlineBar>, // bars[0] is newest, bars[last] is oldest
    pub indicators: IndicatorBundle,
    pub snapshot_ts_local_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PositionContext {
    pub has_position: bool,
    pub symbol: String,
    pub pos_side: String,                  // "long", "short", "none"
    pub pos_size: String,                  // 张数或币数
    pub open_avg_px: Option<f64>,          // 开仓均价
    pub mark_px: Option<f64>,              // 当前标记价
    pub unrealized_pnl: Option<f64>,       // 未实现盈亏 (USDT)
    pub unrealized_pnl_ratio: Option<f64>, // 浮盈比例 (%)
    pub leverage: Option<f64>,
    pub mgn_mode: String,
    pub open_time_ms: Option<i64>,
    pub current_sl: Option<f64>,           // 当前生效的硬止损价
    pub current_tp: Option<f64>,           // 当前生效的硬止盈价
    pub algo_id: Option<String>,           // 绑定的策略委托/止损单 ID
}

