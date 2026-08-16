use crate::data::base::{KlineBar, KlineFrame};
use serde::{Deserialize, Serialize};

pub fn bar_candle_direction_label(bar: &KlineBar) -> &'static str {
    if bar.close > bar.open {
        "阳线"
    } else if bar.close < bar.open {
        "阴线"
    } else {
        "平"
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KlineGeometryFeature {
    pub seq: usize,
    pub bar_type: String,
    pub body_ratio: Option<f64>,
    pub upper_wick_ratio: Option<f64>,
    pub lower_wick_ratio: Option<f64>,
    pub close_position: Option<f64>,
    pub range_atr_ratio: Option<f64>,
    pub ema_relation: String,
    pub overlap_prev_ratio: Option<f64>,
    pub inside_sequence: String,
    pub ioi_pattern: bool,
    pub micro_double: String,
    pub gap_bar: String,
    pub ema_gap_count: usize,
    pub breakout_prev: String,
    pub follow_through_1_2: String,
}

pub fn compute_kline_geometry_features(
    frame: &KlineFrame,
    limit: Option<usize>,
) -> Vec<KlineGeometryFeature> {
    let bars = &frame.bars;
    let mut features = Vec::with_capacity(bars.len());

    for idx in 0..bars.len() {
        let atr = if idx < frame.indicators.atr14.len() {
            frame.indicators.atr14[idx]
        } else {
            f64::NAN
        };
        let ema = if idx < frame.indicators.ema20.len() {
            frame.indicators.ema20[idx]
        } else {
            f64::NAN
        };
        let prev = bars.get(idx + 1);
        let prev2 = bars.get(idx + 2);
        let prev3 = bars.get(idx + 3);

        features.push(feature_for_bar(
            &bars[idx],
            prev,
            prev2,
            prev3,
            bars,
            idx,
            atr,
            ema,
            &frame.indicators.ema20,
        ));
    }

    if let Some(lim) = limit {
        if features.len() > lim {
            features.truncate(lim);
        }
    }
    features
}

fn feature_for_bar(
    bar: &KlineBar,
    prev: Option<&KlineBar>,
    prev2: Option<&KlineBar>,
    prev3: Option<&KlineBar>,
    bars: &[KlineBar],
    idx: usize,
    atr: f64,
    ema: f64,
    ema_values: &[f64],
) -> KlineGeometryFeature {
    let high = bar.high.max(bar.low);
    let low = bar.high.min(bar.low);
    let open_ = bar.open;
    let close = bar.close;
    let full_range = high - low;
    let body = (close - open_).abs();

    let (body_ratio, upper_wick_ratio, lower_wick_ratio, close_position) = if full_range > 0.0 {
        (
            Some(body / full_range),
            Some((high - open_.max(close)) / full_range),
            Some((open_.min(close) - low) / full_range),
            Some((0.0f64).max((1.0f64).min((close - low) / full_range))),
        )
    } else {
        (None, None, None, None)
    };

    let range_atr_ratio = if full_range > 0.0 && !atr.is_nan() && atr > 0.0 {
        Some(full_range / atr)
    } else {
        None
    };

    let ema_relation = if !ema.is_nan() {
        if close > ema {
            "above".to_string()
        } else if close < ema {
            "below".to_string()
        } else {
            "touch".to_string()
        }
    } else {
        "unknown".to_string()
    };

    let overlap_prev_ratio = overlap_ratio(bar, prev);
    let bar_type = classify_bar(bar, prev, body_ratio, close_position);
    let inside_sequence = inside_sequence_str(bar, prev, prev2, prev3);
    let ioi_pattern = is_ioi(bar, prev, prev2, prev3);
    let micro_double = micro_double_str(bar, prev, atr);
    let gap_bar = gap_bar_str(bar, ema);
    let ema_gap_count = ema_gap_count_calc(bars, idx, ema_values);
    let breakout_prev = breakout_prev_range_str(bars, idx, 5);
    let follow_through_1_2 = follow_through_1_2_str(bars, idx);

    KlineGeometryFeature {
        seq: bar.seq,
        bar_type,
        body_ratio: round_or_none(body_ratio),
        upper_wick_ratio: round_or_none(upper_wick_ratio),
        lower_wick_ratio: round_or_none(lower_wick_ratio),
        close_position: round_or_none(close_position),
        range_atr_ratio: round_or_none(range_atr_ratio),
        ema_relation,
        overlap_prev_ratio: round_or_none(overlap_prev_ratio),
        inside_sequence,
        ioi_pattern,
        micro_double,
        gap_bar,
        ema_gap_count,
        breakout_prev,
        follow_through_1_2,
    }
}

fn classify_bar(
    bar: &KlineBar,
    prev: Option<&KlineBar>,
    body_ratio: Option<f64>,
    close_position: Option<f64>,
) -> String {
    if let Some(p) = prev {
        if bar.high <= p.high && bar.low >= p.low {
            return "inside".to_string();
        }
        if bar.high >= p.high && bar.low <= p.low {
            return if bar.close >= bar.open {
                "outside_bull".to_string()
            } else {
                "outside_bear".to_string()
            };
        }
    }

    match (body_ratio, close_position) {
        (Some(b), Some(c)) => {
            if b <= 0.25 {
                "doji".to_string()
            } else if bar.close > bar.open && c >= 0.65 {
                "trend_bull".to_string()
            } else if bar.close < bar.open && c <= 0.35 {
                "trend_bear".to_string()
            } else {
                "other".to_string()
            }
        }
        _ => "flat".to_string(),
    }
}

fn is_inside(bar: Option<&KlineBar>, prev: Option<&KlineBar>) -> bool {
    match (bar, prev) {
        (Some(b), Some(p)) => b.high <= p.high && b.low >= p.low,
        _ => false,
    }
}

fn is_outside(bar: Option<&KlineBar>, prev: Option<&KlineBar>) -> bool {
    match (bar, prev) {
        (Some(b), Some(p)) => b.high >= p.high && b.low <= p.low,
        _ => false,
    }
}

fn inside_sequence_str(
    bar: &KlineBar,
    prev: Option<&KlineBar>,
    prev2: Option<&KlineBar>,
    prev3: Option<&KlineBar>,
) -> String {
    if is_inside(Some(bar), prev) && is_inside(prev, prev2) && is_inside(prev2, prev3) {
        "iii".to_string()
    } else if is_inside(Some(bar), prev) && is_inside(prev, prev2) {
        "ii".to_string()
    } else {
        "none".to_string()
    }
}

fn is_ioi(
    bar: &KlineBar,
    prev: Option<&KlineBar>,
    prev2: Option<&KlineBar>,
    prev3: Option<&KlineBar>,
) -> bool {
    is_inside(prev2, prev3) && is_outside(prev, prev2) && is_inside(Some(bar), prev)
}

fn micro_double_str(bar: &KlineBar, prev: Option<&KlineBar>, atr: f64) -> String {
    if let Some(p) = prev {
        let tolerance = if !atr.is_nan() && atr > 0.0 { atr * 0.02 } else { 0.0 };
        if (bar.low - p.low).abs() <= tolerance {
            return "MDB".to_string();
        }
        if (bar.high - p.high).abs() <= tolerance {
            return "MDT".to_string();
        }
    }
    "none".to_string()
}

fn gap_bar_str(bar: &KlineBar, ema: f64) -> String {
    if ema.is_nan() {
        return "none".to_string();
    }
    if bar.low > ema {
        "bull_gap".to_string()
    } else if bar.high < ema {
        "bear_gap".to_string()
    } else {
        "none".to_string()
    }
}

fn ema_gap_count_calc(bars: &[KlineBar], idx: usize, ema_values: &[f64]) -> usize {
    if idx >= ema_values.len() || ema_values[idx].is_nan() {
        return 0;
    }
    let side = gap_bar_str(&bars[idx], ema_values[idx]);
    if side == "none" {
        return 0;
    }
    let mut count = 0;
    for j in idx..bars.len() {
        if j >= ema_values.len() || ema_values[j].is_nan() {
            break;
        }
        if gap_bar_str(&bars[j], ema_values[j]) != side {
            break;
        }
        count += 1;
    }
    count
}

fn breakout_prev_range_str(bars: &[KlineBar], idx: usize, lookback: usize) -> String {
    let end = (idx + 1 + lookback).min(bars.len());
    let prev_bars = &bars[idx + 1..end];
    if prev_bars.is_empty() {
        return "none".to_string();
    }
    let max_high = prev_bars.iter().map(|b| b.high).fold(f64::NEG_INFINITY, f64::max);
    let min_low = prev_bars.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
    let broke_high = bars[idx].high > max_high;
    let broke_low = bars[idx].low < min_low;
    if broke_high && broke_low {
        "both".to_string()
    } else if broke_high {
        "up".to_string()
    } else if broke_low {
        "down".to_string()
    } else {
        "none".to_string()
    }
}

fn follow_through_1_2_str(bars: &[KlineBar], idx: usize) -> String {
    if idx == 0 {
        return "pending".to_string();
    }
    let bar = &bars[idx];
    let start = if idx >= 2 { idx - 2 } else { 0 };
    let newer = &bars[start..idx];
    if newer.is_empty() {
        return "pending".to_string();
    }
    let direction = if bar.close > bar.open { 1 } else if bar.close < bar.open { -1 } else { 0 };
    if direction == 0 {
        return "pending".to_string();
    }
    let mut same = 0;
    let mut opposite = 0;
    for nbar in newer {
        if direction > 0 {
            if nbar.close > bar.close { same += 1; }
            if nbar.close < bar.open { opposite += 1; }
        } else {
            if nbar.close < bar.close { same += 1; }
            if nbar.close > bar.open { opposite += 1; }
        }
    }
    if same > 0 {
        "yes".to_string()
    } else if opposite > 0 {
        "failed".to_string()
    } else {
        "no".to_string()
    }
}

fn overlap_ratio(bar: &KlineBar, prev: Option<&KlineBar>) -> Option<f64> {
    let p = prev?;
    let high = bar.high.min(p.high);
    let low = bar.low.max(p.low);
    let overlap = (0.0f64).max(high - low);
    let denominator = bar.high.max(p.high) - bar.low.min(p.low);
    if denominator <= 0.0 {
        return None;
    }
    Some(overlap / denominator)
}

fn round_or_none(value: Option<f64>) -> Option<f64> {
    value.and_then(|v| {
        if v.is_nan() {
            None
        } else {
            Some((v * 1000.0).round() / 1000.0)
        }
    })
}
