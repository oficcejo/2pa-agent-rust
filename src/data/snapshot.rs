use crate::data::bar_close::has_forming_bar_at_head;
use crate::data::base::{IndicatorBundle, KlineBar, KlineFrame};
use crate::indicators::atr::atr_full;
use crate::indicators::ema::ema_full;
use crate::util::timefmt::now_local_ms;

pub const INDICATOR_WARMUP_BARS: usize = 50;

pub fn compute_indicators(bars: &[KlineBar]) -> IndicatorBundle {
    let mut bars_asc: Vec<KlineBar> = bars.to_vec();
    bars_asc.reverse();

    let closes: Vec<f64> = bars_asc.iter().map(|b| b.close).collect();
    let highs: Vec<f64> = bars_asc.iter().map(|b| b.high).collect();
    let lows: Vec<f64> = bars_asc.iter().map(|b| b.low).collect();

    let mut ema20_asc = ema_full(&closes, 20);
    let mut atr14_asc = atr_full(&highs, &lows, &closes, 14);

    ema20_asc.reverse();
    atr14_asc.reverse();

    IndicatorBundle {
        ema20: ema20_asc,
        atr14: atr14_asc,
    }
}

pub fn build_analysis_frame(
    bars_raw: &[KlineBar],
    n: usize,
    symbol: &str,
    timeframe: &str,
    now_ms: Option<i64>,
) -> Option<KlineFrame> {
    let forming = has_forming_bar_at_head(bars_raw, Some(timeframe), now_ms);
    let avail_closed = if forming {
        if bars_raw.is_empty() { 0 } else { bars_raw.len() - 1 }
    } else {
        bars_raw.len()
    };

    if avail_closed < n {
        return None;
    }

    let fetch_n = (n + INDICATOR_WARMUP_BARS).min(avail_closed);
    let start_idx = if forming { 1 } else { 0 };
    let closed_raw = &bars_raw[start_idx..start_idx + fetch_n];

    let mut rebased_all = Vec::with_capacity(closed_raw.len());
    for (i, b) in closed_raw.iter().enumerate() {
        let mut bar = b.normalized();
        bar.seq = i + 1;
        bar.closed = true;
        rebased_all.push(bar);
    }

    let indicators_all = compute_indicators(&rebased_all);
    let rebased = rebased_all[..n].to_vec();
    let indicators = IndicatorBundle {
        ema20: indicators_all.ema20[..n].to_vec(),
        atr14: indicators_all.atr14[..n].to_vec(),
    };

    Some(KlineFrame {
        symbol: symbol.to_string(),
        timeframe: timeframe.to_string(),
        bars: rebased,
        indicators,
        snapshot_ts_local_ms: now_local_ms(),
    })
}

pub fn build_live_frame(
    bars_raw: &[KlineBar],
    n_closed: usize,
    symbol: &str,
    timeframe: &str,
    now_ms: Option<i64>,
) -> Option<KlineFrame> {
    let has_forming = has_forming_bar_at_head(bars_raw, Some(timeframe), now_ms);
    let raw = if has_forming {
        if bars_raw.len() < n_closed + 1 {
            return None;
        }
        &bars_raw[..n_closed + 1]
    } else {
        if bars_raw.len() < n_closed {
            return None;
        }
        &bars_raw[..n_closed]
    };

    let mut rebased = Vec::with_capacity(raw.len());
    let mut closed_idx = 0;
    for (i, b) in raw.iter().enumerate() {
        let is_forming = has_forming && i == 0;
        let seq = if is_forming {
            0
        } else {
            closed_idx += 1;
            closed_idx
        };
        let mut bar = b.normalized();
        bar.seq = seq;
        bar.closed = !is_forming;
        rebased.push(bar);
    }

    let indicators = compute_indicators(&rebased);
    Some(KlineFrame {
        symbol: symbol.to_string(),
        timeframe: timeframe.to_string(),
        bars: rebased,
        indicators,
        snapshot_ts_local_ms: now_local_ms(),
    })
}
