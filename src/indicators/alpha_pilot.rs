//! AlphaPilot Quantitative Factor Strategy Engine
//!
//! Ported directly from okx-alpha-pilot (checkpoints/best_ETH-USDT-SWAP_15m.json).
//!
//! Formula: RS_VOL -> JUMP -> SIGN -> SUPERTREND_DIR -> POWER -> NEG -> SIGN -> MAX
//! Normalization: 500-bar rolling causal Z-score (with expanding fallback for T < 500).
//! Mapping: Neutral Band [0.25, 0.75] stateless position compression into [-1.0, 1.0].

use crate::data::base::KlineBar;
use serde::{Deserialize, Serialize};

pub const DEFAULT_ROLL_WINDOW: usize = 500;
pub const LOWER_BAND: f64 = 0.25;
pub const UPPER_BAND: f64 = 0.75;
pub const RS_ROBUST_WINDOW: usize = 200;
pub const RS_SMOOTH_WINDOW: usize = 20;
pub const SUPERTREND_ATR_WINDOW: usize = 14;
pub const SUPERTREND_MULTIPLIER: f64 = 1.5;

/// Full AlphaPilot evaluation output for a single analysis step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlphaPilotResult {
    pub raw_factor: f64,
    pub z_score: f64,
    pub target_position: f64,
    pub supertrend_dir: f64,
    pub supertrend_upper: f64,
    pub supertrend_lower: f64,
    pub rs_vol_norm: f64,
    pub is_vol_jump: bool,
    pub action_label: String,
    pub order_action: String,       // "OPEN", "HOLD", "CLOSE_EARLY", "WAIT"
    pub order_direction: Option<String>, // "做多", "做空", None
    pub entry_price: f64,
    pub stop_loss_price: Option<f64>,
    pub take_profit_price: Option<f64>,
    pub confidence: f64,            // 0.0 ~ 1.0
    pub rationale: String,
}

/// Computes lower median (matching PyTorch's torch.median: element at (len-1)//2).
fn torch_median(slice: &[f64]) -> f64 {
    if slice.is_empty() {
        return 0.0;
    }
    let mut sorted = slice.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = (sorted.len() - 1) / 2;
    sorted[idx]
}

/// Rogers-Satchell Volatility (R1.3)
///
/// Per bar: RS = ln(H/C)*ln(H/O) + ln(L/C)*ln(L/O)
/// Rolling mean (w=20) padded with 19 zeros at front.
/// raw_vol = sqrt(max(mean, 0.0))
/// norm(log1p(clean(raw_vol))) using 200-bar causal median/MAD normalization clipped to [-5, 5].
pub fn compute_rs_vol(highs: &[f64], lows: &[f64], opens: &[f64], closes: &[f64]) -> Vec<f64> {
    let t = closes.len();
    if t == 0 {
        return Vec::new();
    }
    let eps = 1e-9;

    // 1. Per-bar RS value
    let mut rs_bar = Vec::with_capacity(t);
    for i in 0..t {
        let h = highs[i] + eps;
        let l = lows[i] + eps;
        let o = opens[i] + eps;
        let c = closes[i] + eps;
        let val = (h / c).ln() * (h / o).ln() + (l / c).ln() * (l / o).ln();
        rs_bar.push(if val.is_nan() { 0.0 } else { val });
    }

    // 2. Rolling mean 20 with 19 zeros padded at the front
    let w_smooth = RS_SMOOTH_WINDOW;
    let mut padded_rs = vec![0.0; w_smooth - 1];
    padded_rs.extend_from_slice(&rs_bar);

    let mut log1p_vol = Vec::with_capacity(t);
    let mut running_sum = 0.0;
    for (i, &val) in padded_rs.iter().enumerate() {
        running_sum += val;
        if i >= w_smooth {
            running_sum -= padded_rs[i - w_smooth];
        }
        if i >= w_smooth - 1 {
            let mean = running_sum / (w_smooth as f64);
            let raw_vol = mean.max(0.0).sqrt();
            log1p_vol.push((1.0 + raw_vol).ln());
        }
    }

    // 3. Robust norm with window 200 (median/MAD)
    let w_norm = RS_ROBUST_WINDOW;
    let mut padded_log = vec![0.0; w_norm - 1];
    padded_log.extend_from_slice(&log1p_vol);

    let mut rs_norm = Vec::with_capacity(t);
    for i in 0..t {
        let wnd = &padded_log[i..i + w_norm];
        let med = torch_median(wnd);

        // MAD
        let mut dev = Vec::with_capacity(w_norm);
        for &x in wnd {
            dev.push((x - med).abs());
        }
        let mad = torch_median(&dev) + 1e-6;

        let z = (log1p_vol[i] - med) / mad;
        let clamped = z.clamp(-5.0, 5.0);
        rs_norm.push(if clamped.is_nan() { 0.0 } else { clamped });
    }

    rs_norm
}

/// SuperTrend Direction {-1.0, +1.0}
///
/// upper_band = (high + low)/2 + 1.5 * ATR14 (previous bar)
/// lower_band = (high + low)/2 - 1.5 * ATR14
/// t=0: direction = +1
/// t>=1: close > prev_upper => +1, close < prev_lower => -1, else retain
pub fn compute_supertrend_dir(
    highs: &[f64],
    lows: &[f64],
    closes: &[f64],
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let t = closes.len();
    if t == 0 {
        return (Vec::new(), Vec::new(), Vec::new());
    }

    // 1. True Range
    let mut tr = Vec::with_capacity(t);
    for i in 0..t {
        let prev_c = if i > 0 { closes[i - 1] } else { closes[0] };
        let h = highs[i];
        let l = lows[i];
        let hl = h - l;
        let hc = (h - prev_c).abs();
        let lc = (l - prev_c).abs();
        tr.push(hl.max(hc).max(lc));
    }

    // 2. Rolling mean 14 with 13 zeros padded at front
    let w_atr = SUPERTREND_ATR_WINDOW;
    let mut padded_tr = vec![0.0; w_atr - 1];
    padded_tr.extend_from_slice(&tr);

    let mut atr = Vec::with_capacity(t);
    let mut running_sum = 0.0;
    for (i, &val) in padded_tr.iter().enumerate() {
        running_sum += val;
        if i >= w_atr {
            running_sum -= padded_tr[i - w_atr];
        }
        if i >= w_atr - 1 {
            atr.push(running_sum / (w_atr as f64));
        }
    }

    // 3. Bands
    let mut upper_band = Vec::with_capacity(t);
    let mut lower_band = Vec::with_capacity(t);
    for i in 0..t {
        let mid = (highs[i] + lows[i]) / 2.0;
        upper_band.push(mid + SUPERTREND_MULTIPLIER * atr[i]);
        lower_band.push(mid - SUPERTREND_MULTIPLIER * atr[i]);
    }

    // 4. Direction
    let mut direction = Vec::with_capacity(t);
    direction.push(1.0);
    let mut prev_upper = upper_band[0];
    let mut prev_lower = lower_band[0];

    for i in 1..t {
        let c = closes[i];
        let prev_dir = direction[i - 1];
        let new_dir = if c > prev_upper {
            1.0
        } else if c < prev_lower {
            -1.0
        } else {
            prev_dir
        };
        direction.push(new_dir);
        prev_upper = upper_band[i];
        prev_lower = lower_band[i];
    }

    (direction, upper_band, lower_band)
}

/// JUMP operator: tanh(z - 1.5) where z = (x - mean) / (std + 1e-6)
pub fn op_jump(x: &[f64]) -> Vec<f64> {
    let t = x.len();
    if t == 0 {
        return Vec::new();
    }
    // Expanding Welford statistics: every output depends only on its prefix.
    let mut mean = 0.0;
    let mut m2 = 0.0;
    x.iter().enumerate().map(|(i, &v)| {
        let count = (i + 1) as f64;
        let delta = v - mean;
        mean += delta / count;
        m2 += delta * (v - mean);
        let variance = if i > 0 { m2 / i as f64 } else { 0.0 };
        ((v - mean) / (variance.max(0.0).sqrt() + 1e-6) - 1.5).tanh()
    }).collect()
}

/// Computes the raw factor for the AlphaPilot strategy:
/// MAX(SIGN(JUMP(RS_VOL)), -SUPERTREND_DIR)
pub fn compute_raw_factor(
    highs: &[f64],
    lows: &[f64],
    opens: &[f64],
    closes: &[f64],
) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
    let rs_vol = compute_rs_vol(highs, lows, opens, closes);
    let (st_dir, upper_band, lower_band) = compute_supertrend_dir(highs, lows, closes);
    let jump = op_jump(&rs_vol);

    let t = closes.len();
    let mut raw_factor = Vec::with_capacity(t);

    for i in 0..t {
        let sign_jump: f64 = if jump[i] > 0.0 {
            1.0
        } else if jump[i] < 0.0 {
            -1.0
        } else {
            0.0
        };

        // -SUPERTREND_DIR
        let sign_neg_st = -st_dir[i];

        let max_val = sign_jump.max(sign_neg_st);
        raw_factor.push(max_val);
    }

    (raw_factor, rs_vol, st_dir, upper_band, lower_band)
}

/// 500-bar rolling causal Z-score normalization matching StackVM._normalize_output.
/// Uses expanding fallback when T < 500.
/// Uses sample standard deviation (ddof=1, N-1).
/// Expands causally during warm-up; returns zero for fewer than two values or std < 1e-4.
/// Clips result to [-3.0, 3.0].
pub fn rolling_zscore_500(x: &[f64]) -> Vec<f64> {
    x.iter().enumerate().map(|(i, &value)| {
        let window = &x[i.saturating_sub(DEFAULT_ROLL_WINDOW - 1)..=i];
        if window.len() < 2 { return 0.0; }
        let mean = window.iter().sum::<f64>() / window.len() as f64;
        let std = (window.iter().map(|v| (v-mean).powi(2)).sum::<f64>() / (window.len()-1) as f64).sqrt();
        if std < 1e-4 { 0.0 } else { ((value-mean)/std).clamp(-3.0,3.0) }
    }).collect()
}

/// Stateless target position mapping via tanh & Neutral Band [0.25, 0.75].
/// Returns position in [-1.0, 1.0].
pub fn compute_target_positions_stateless(factors: &[f64]) -> Vec<f64> {
    factors
        .iter()
        .map(|&f| {
            let raw = f.tanh();
            let abs_raw = raw.abs();
            let scale = ((abs_raw - LOWER_BAND) / (UPPER_BAND - LOWER_BAND)).clamp(0.0, 1.0);
            raw * scale
        })
        .collect()
}

/// Convert continuous position [-1.0, 1.0] to readable label.
pub fn signal_to_action(signal: f64) -> String {
    if signal > 0.05 {
        format!("做多 {:.1}%", signal * 100.0)
    } else if signal < -0.05 {
        format!("做空 {:.1}%", signal.abs() * 100.0)
    } else {
        "空仓观望".to_string()
    }
}

/// AlphaPilot Quantitative Strategy Engine
pub struct AlphaPilotEngine;

impl AlphaPilotEngine {
    /// Evaluates K-lines (bars in chronological or reverse order) and returns the latest AlphaPilotResult.
    /// bars can be passed directly from KlineFrame (where bars[0] is newest).
    pub fn evaluate(bars: &[KlineBar]) -> Option<AlphaPilotResult> {
        if bars.is_empty() {
            return None;
        }

        // Convert to chronological order (oldest to newest)
        let mut bars_chrono: Vec<KlineBar> = bars.to_vec();
        if bars.len() > 1 && bars[0].ts_open > bars[1].ts_open {
            bars_chrono.reverse();
        }

        let closes: Vec<f64> = bars_chrono.iter().map(|b| b.close).collect();
        let highs: Vec<f64> = bars_chrono.iter().map(|b| b.high).collect();
        let lows: Vec<f64> = bars_chrono.iter().map(|b| b.low).collect();
        let opens: Vec<f64> = bars_chrono.iter().map(|b| b.open).collect();

        let (raw_factors, rs_vol, st_dir, upper_band, lower_band) =
            compute_raw_factor(&highs, &lows, &opens, &closes);
        let z_scores = rolling_zscore_500(&raw_factors);
        let target_positions = compute_target_positions_stateless(&z_scores);

        let last_idx = closes.len() - 1;
        let last_pos = target_positions[last_idx];
        let last_z = z_scores[last_idx];
        let last_raw = raw_factors[last_idx];
        let last_st_dir = st_dir[last_idx];
        let last_upper = upper_band[last_idx];
        let last_lower = lower_band[last_idx];
        let last_rs = rs_vol[last_idx];
        let last_close = closes[last_idx];

        let is_vol_jump = last_rs > 1.5;
        let action_label = signal_to_action(last_pos);

        let (order_action, order_direction, stop_loss_price, take_profit_price) = if last_pos > 0.05 {
            let sl = last_lower.min(last_close * 0.985);
            let risk = (last_close - sl).abs().max(last_close * 0.005);
            let tp = last_close + 2.0 * risk;
            ("OPEN".to_string(), Some("做多".to_string()), Some(sl), Some(tp))
        } else if last_pos < -0.05 {
            let sl = last_upper.max(last_close * 1.015);
            let risk = (sl - last_close).abs().max(last_close * 0.005);
            let tp = last_close - 2.0 * risk;
            ("OPEN".to_string(), Some("做空".to_string()), Some(sl), Some(tp))
        } else {
            ("WAIT".to_string(), None, None, None)
        };

        let confidence = (last_pos.abs() * 100.0).clamp(0.0, 100.0) / 100.0;

        let rationale = format!(
            "AlphaPilot 符号量化因子评估: 滚动标准化 Z-Score={:.4}, 目标仓位={:.2} ({})。\n\
             - Rogers-Satchell 波动率归一化: {:.4} (异动跳跃: {})\n\
             - SuperTrend 趋势状态: {} (上轨: {:.2}, 下轨: {:.2})\n\
             - 中性过滤状态: {} (下限 0.25, 上限 0.75)",
            last_z,
            last_pos,
            action_label,
            last_rs,
            if is_vol_jump { "触发超额跳跃 (>1.5σ)" } else { "常态" },
            if last_st_dir > 0.0 { "多头趋势 (1.0)" } else { "空头趋势 (-1.0)" },
            last_upper,
            last_lower,
            if last_pos.abs() < 1e-4 { "位于噪声中性区间，主动保持空仓" } else { "已突破中性区间，有效入场" }
        );

        Some(AlphaPilotResult {
            raw_factor: last_raw,
            z_score: last_z,
            target_position: last_pos,
            supertrend_dir: last_st_dir,
            supertrend_upper: last_upper,
            supertrend_lower: last_lower,
            rs_vol_norm: last_rs,
            is_vol_jump,
            action_label,
            order_action,
            order_direction,
            entry_price: last_close,
            stop_loss_price,
            take_profit_price,
            confidence,
            rationale,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_torch_median_even_and_odd() {
        let even = vec![10.0, 20.0, 30.0, 40.0];
        assert_eq!(torch_median(&even), 20.0);

        let odd = vec![10.0, 20.0, 30.0];
        assert_eq!(torch_median(&odd), 20.0);
    }

    #[test]
    fn test_neutral_band_mapping() {
        // Below 0.25 => 0.0
        let low = vec![0.1, -0.2];
        let pos_low = compute_target_positions_stateless(&low);
        assert_eq!(pos_low[0], 0.0);
        assert_eq!(pos_low[1], 0.0);

        // High factor => saturated near 1.0 or -1.0
        let high = vec![3.0, -3.0];
        let pos_high = compute_target_positions_stateless(&high);
        assert!(pos_high[0] > 0.95);
        assert!(pos_high[1] < -0.95);
    }

    #[test]
    fn test_supertrend_logic() {
        let highs = vec![105.0, 106.0, 107.0, 102.0];
        let lows = vec![95.0, 96.0, 97.0, 91.0];
        let closes = vec![100.0, 102.0, 105.0, 92.0];

        let (dir, upper, lower) = compute_supertrend_dir(&highs, &lows, &closes);
        assert_eq!(dir.len(), 4);
        assert_eq!(upper.len(), 4);
        assert_eq!(lower.len(), 4);
        assert_eq!(dir[0], 1.0);
    }
}
