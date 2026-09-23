//! Historical data feeds: loading from files/fixtures and OKX API pagination with gap-checking.

use crate::data::base::KlineBar;
use crate::okx::client::OKXClient;
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use tracing::{info, warn};

/// Convert timeframe string to milliseconds.
pub fn timeframe_to_ms(tf: &str) -> i64 {
    match tf.trim().to_lowercase().as_str() {
        "1m" => 60_000,
        "3m" => 180_000,
        "5m" => 300_000,
        "15m" => 900_000,
        "30m" => 1_800_000,
        "1h" | "1H" => 3_600_000,
        "2h" | "2H" => 7_200_000,
        "4h" | "4H" => 14_400_000,
        "6h" | "6H" => 21_600_000,
        "12h" | "12H" => 43_200_000,
        "1d" | "1D" => 86_400_000,
        _ => 900_000, // default 15m
    }
}

/// Information about a detected gap in historical candles.
#[derive(Debug, Clone)]
pub struct DataGap {
    pub prev_ts: i64,
    pub next_ts: i64,
    pub missing_bars: usize,
}

/// Inspect a slice of sorted ascending bars for gaps.
pub fn detect_gaps(bars: &[KlineBar], expected_interval_ms: i64) -> Vec<DataGap> {
    let mut gaps = Vec::new();
    if bars.len() < 2 || expected_interval_ms <= 0 {
        return gaps;
    }

    for i in 1..bars.len() {
        let delta = bars[i].ts_open - bars[i - 1].ts_open;
        if delta > expected_interval_ms {
            let missing = (delta / expected_interval_ms).saturating_sub(1) as usize;
            if missing > 0 {
                gaps.push(DataGap {
                    prev_ts: bars[i - 1].ts_open,
                    next_ts: bars[i].ts_open,
                    missing_bars: missing,
                });
            }
        }
    }
    gaps
}

/// Parse raw OKX candle array `[ts, o, h, l, c, vol, ...]` into normalized KlineBar.
pub fn parse_okx_candle_row(row: &[String]) -> Option<KlineBar> {
    if row.len() < 6 {
        return None;
    }

    let ts_open = row[0].parse::<i64>().ok()?;
    let open = row[1].parse::<f64>().ok()?;
    let high_raw = row[2].parse::<f64>().ok()?;
    let low_raw = row[3].parse::<f64>().ok()?;
    let close_raw = row[4].parse::<f64>().ok()?;
    let volume = row[5].parse::<f64>().unwrap_or(0.0).max(0.0);

    let high = high_raw.max(low_raw).max(open).max(close_raw);
    let low = low_raw.min(high_raw).min(open).min(close_raw);
    let close = close_raw.clamp(low, high);
    let open = open.clamp(low, high);

    Some(KlineBar {
        seq: 0,
        ts_open,
        open,
        high,
        low,
        close,
        volume,
        amount: 0.0,
        pct_chg: None,
        closed: true,
    })
}

/// Load historical candles from a local JSON file fixture.
/// Supports both OKX array-of-arrays format and KlineBar object format.
pub fn load_candles_from_file(path: &Path) -> Result<Vec<KlineBar>> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open candle fixture file {:?}", path))?;
    let reader = BufReader::new(file);
    let json_val: Value = serde_json::from_reader(reader)
        .with_context(|| format!("Failed to parse JSON from {:?}", path))?;

    let mut map: BTreeMap<i64, KlineBar> = BTreeMap::new();

    if let Some(arr) = json_val.as_array() {
        for item in arr {
            if let Some(row) = item.as_array() {
                let string_row: Vec<String> = row
                    .iter()
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        _ => String::new(),
                    })
                    .collect();
                if let Some(bar) = parse_okx_candle_row(&string_row) {
                    map.insert(bar.ts_open, bar);
                }
            } else if let Ok(bar) = serde_json::from_value::<KlineBar>(item.clone()) {
                map.insert(bar.ts_open, bar.normalized());
            }
        }
    } else if let Some(bars_val) = json_val.get("bars").and_then(|v| v.as_array()) {
        for item in bars_val {
            if let Ok(bar) = serde_json::from_value::<KlineBar>(item.clone()) {
                map.insert(bar.ts_open, bar.normalized());
            }
        }
    } else {
        return Err(anyhow!("Unrecognized JSON structure in candle fixture {:?}", path));
    }

    let mut result: Vec<KlineBar> = map.into_values().collect();
    // Sort ascending chronologically (oldest to newest)
    result.sort_by_key(|b| b.ts_open);

    info!("Loaded {} bars from file {:?}", result.len(), path);
    Ok(result)
}

/// Fetch historical candles from OKX with pagination, gap-checking, and timestamp normalization.
pub async fn fetch_candles_okx(
    client: &OKXClient,
    inst_id: &str,
    timeframe: &str,
    total: usize,
    end_time_ms: Option<i64>,
) -> Result<Vec<KlineBar>> {
    info!(
        "Fetching up to {} historical candles for {} ({}) from OKX (end_time_ms={:?})...",
        total, inst_id, timeframe, end_time_ms
    );

    let interval_ms = timeframe_to_ms(timeframe);
    let initial_after = end_time_ms.map(|ts| (ts + interval_ms).to_string());

    // Call paginated client method
    let rows = client
        .get_candles_paginated_with_cursor(
            inst_id,
            timeframe,
            total,
            true,
            initial_after.as_deref(),
        )
        .await
        .with_context(|| format!("Failed to fetch candles for {} from OKX", inst_id))?;

    let mut map: BTreeMap<i64, KlineBar> = BTreeMap::new();
    for r in rows {
        if let Some(bar) = parse_okx_candle_row(&r) {
            map.insert(bar.ts_open, bar);
        }
    }

    let mut bars: Vec<KlineBar> = map.into_values().collect();
    bars.sort_by_key(|b| b.ts_open);

    let interval_ms = timeframe_to_ms(timeframe);
    let gaps = detect_gaps(&bars, interval_ms);
    if !gaps.is_empty() {
        warn!(
            "Detected {} data gap(s) in historical candles for {}: total missing bars ~{}",
            gaps.len(),
            inst_id,
            gaps.iter().map(|g| g.missing_bars).sum::<usize>()
        );
    }

    info!("Retrieved {} normalized ascending bars from OKX", bars.len());
    Ok(bars)
}

/// Generate synthetic candles for deterministic unit tests and simulations.
pub fn generate_synthetic_candles(
    count: usize,
    start_price: f64,
    interval_ms: i64,
    start_ts_ms: i64,
) -> Vec<KlineBar> {
    generate_synthetic_candles_for_strategy("all", count, start_price, interval_ms, start_ts_ms)
}

/// Generate synthetic candles tailored for a specific strategy or a multi-regime combination of all strategies.
pub fn generate_synthetic_candles_for_strategy(
    strategy: &str,
    count: usize,
    start_price: f64,
    interval_ms: i64,
    start_ts_ms: i64,
) -> Vec<KlineBar> {
    let mut bars = Vec::with_capacity(count);

    if count < 50 {
        let mut price = start_price;
        for i in 0..count {
            let ts = start_ts_ms + (i as i64) * interval_ms;
            let cycle = (i as f64 * 0.1).sin();
            let delta = cycle * (price * 0.005);
            let open = price;
            let close = (price + delta).max(10.0);
            let high = open.max(close) + (price * 0.002);
            let low = open.min(close) - (price * 0.002);
            let volume = 100.0 + (i % 10) as f64 * 10.0;
            bars.push(KlineBar {
                seq: 0,
                ts_open: ts,
                open,
                high,
                low,
                close,
                volume,
                amount: volume * close,
                pct_chg: Some((close - open) / open * 100.0),
                closed: true,
            });
            price = close;
        }
        return bars;
    }

    let strat_clean = strategy.trim().to_lowercase();
    let is_dog_reversion = strat_clean == "dog_reversion" || strat_clean == "dog_walking" || strat_clean == "遛狗";
    let is_dog_trend = strat_clean == "dog_trend";
    let is_2pa = strat_clean == "2pa" || strat_clean == "2pa_trend" || strat_clean == "2pa_source";

    let mut current_open = start_price;
    for i in 0..count {
        let ts = start_ts_ms + (i as i64) * interval_ms;
        let (open, high, low, close) = if i < 200 {
            // Warmup phase (bars 0..199):
            // Establish SMA170, EMA20, ATR14, plus strong swing highs and lows for future structural obstacles
            let t = i as f64;
            let trend_drift = (t / 200.0) * 0.06; // +6% upward drift ensures SMA170 slope >= 0.02 and HTF uptrend
            let macro_wave = (t * 0.01).sin() * 0.02;
            let micro_wave = (t * 0.25).sin() * 0.005;
            let target_c = start_price * (0.97 + trend_drift + macro_wave + micro_wave);
            let o = current_open;
            let c = target_c;
            let extra_h = if (35..=45).contains(&i) || (135..=145).contains(&i) {
                start_price * 0.04
            } else {
                start_price * 0.004
            };
            let extra_l = if (85..=95).contains(&i) {
                start_price * 0.03
            } else {
                start_price * 0.004
            };
            let h = o.max(c) + extra_h;
            let l = (o.min(c) - extra_l).max(10.0);
            (o, h, l, c)
        } else {
            // Trading simulation phase (bars 200..):
            // Multi-regime or targeted strategy execution
            let sub_regime = if is_dog_reversion {
                2
            } else if is_dog_trend {
                1
            } else if is_2pa {
                0
            } else {
                ((i - 200) / 35) % 3
            };

            let step = (i - 200) % 35;
            let cycle_idx = (i - 200) / 35;
            let base_offset = (cycle_idx as f64 * 0.003) * start_price;
            let p_base = start_price * 1.045 + base_offset;
            let o = current_open;

            let (h, l, c) = match sub_regime {
                // Regime 0: 2PA Trend setups (H2 pullbacks to EMA20)
                0 => match step {
                    0..=12 => {
                        let progress = (step as f64 + 1.0) / 13.0;
                        let c_val = p_base * (1.0 + progress * 0.02);
                        (o.max(c_val) + p_base * 0.003, o.min(c_val) - p_base * 0.002, c_val)
                    }
                    13 => {
                        let c_val = p_base * 1.022;
                        (p_base * 1.026, p_base * 1.018, c_val)
                    }
                    14 => {
                        let c_val = p_base * 1.016;
                        (p_base * 1.023, p_base * 1.015, c_val)
                    }
                    15 => {
                        let c_val = p_base * 1.021;
                        (p_base * 1.025, p_base * 1.015, c_val)
                    }
                    16 => {
                        let c_val = p_base * 1.014;
                        (p_base * 1.022, p_base * 1.012, c_val)
                    }
                    17 => {
                        let c_val = p_base * 1.013;
                        (p_base * 1.017, p_base * 1.008, c_val)
                    }
                    18 => {
                        // Bullish 2PA confirmation bar (H2)
                        let c_val = p_base * 1.027;
                        (p_base * 1.028, p_base * 1.010, c_val)
                    }
                    19..=25 => {
                        let progress = (step - 19 + 1) as f64 / 7.0;
                        let c_val = p_base * (1.026 + progress * 0.04);
                        (o.max(c_val) + p_base * 0.006, o.min(c_val) - p_base * 0.002, c_val)
                    }
                    _ => {
                        let progress = (step - 26 + 1) as f64 / 9.0;
                        let c_val = p_base * (1.066 - progress * 0.045);
                        (o.max(c_val) + p_base * 0.003, o.min(c_val) - p_base * 0.003, c_val)
                    }
                },
                // Regime 1: Dog Trend setups (Retest of rising SMA170 and bounce)
                1 => match step {
                    0..=10 => {
                        let progress = (step as f64 + 1.0) / 11.0;
                        let c_val = p_base * (1.005 + progress * 0.025);
                        (o.max(c_val) + p_base * 0.003, o.min(c_val) - p_base * 0.002, c_val)
                    }
                    11..=12 => {
                        // Rapid pullback toward rising SMA170 (~start_price * 1.018)
                        let progress = (step - 11 + 1) as f64 / 2.0;
                        let c_val = p_base * 1.03 - progress * (p_base * 1.03 - start_price * 1.018);
                        (o.max(c_val) + p_base * 0.002, o.min(c_val) - p_base * 0.002, c_val)
                    }
                    13 => {
                        // b[1] touches SMA170 (SMA170 is around start_price * 1.018)
                        let c_val = start_price * 1.018;
                        let l_val = start_price * 1.015;
                        let h_val = start_price * 1.021;
                        (h_val, l_val, c_val)
                    }
                    14 => {
                        // b[0] strong bullish bounce off SMA170
                        let c_val = start_price * 1.036;
                        let h_val = start_price * 1.038;
                        let l_val = o.min(start_price * 1.016);
                        (h_val, l_val, c_val)
                    }
                    15..=24 => {
                        let progress = (step - 15 + 1) as f64 / 10.0;
                        let c_val = start_price * (1.036 + progress * 0.04);
                        (o.max(c_val) + p_base * 0.006, o.min(c_val) - p_base * 0.002, c_val)
                    }
                    _ => {
                        let progress = (step - 25 + 1) as f64 / 10.0;
                        let c_val = start_price * (1.076 - progress * 0.04);
                        (o.max(c_val) + p_base * 0.003, o.min(c_val) - p_base * 0.003, c_val)
                    }
                },
                // Regime 2: Dog Reversion setups (Deep plunge >2.5 ATR from SMA170, double bottom, SMA14 reclaim)
                _ => match step {
                    0..=8 => {
                        // Deep selloff plunge away from SMA170 (>2.5 ATR)
                        let progress = (step as f64 + 1.0) / 9.0;
                        let c_val = p_base * (1.000 - progress * 0.06);
                        (o.max(c_val) + p_base * 0.002, o.min(c_val) - p_base * 0.003, c_val)
                    }
                    9 => {
                        // First extreme low at -3.5 ATR
                        let c_val = p_base * 0.938;
                        let l_val = p_base * 0.935;
                        let h_val = p_base * 0.942;
                        (h_val, l_val, c_val)
                    }
                    10 => {
                        // Small bounce
                        let c_val = p_base * 0.946;
                        (p_base * 0.948, p_base * 0.941, c_val)
                    }
                    11 => {
                        // Deceleration pause
                        let c_val = p_base * 0.943;
                        (p_base * 0.946, p_base * 0.941, c_val)
                    }
                    12 => {
                        // b[1]: Second test of the extreme low
                        let c_val = p_base * 0.940;
                        let l_val = p_base * 0.936;
                        let h_val = p_base * 0.944;
                        (h_val, l_val, c_val)
                    }
                    13 => {
                        // b[0]: Bullish reversal candle reclaiming SMA14 with >2.5 ATR room to SMA170
                        let c_val = p_base * 0.962;
                        let h_val = p_base * 0.965;
                        let l_val = o.min(p_base * 0.938);
                        (h_val, l_val, c_val)
                    }
                    14..=22 => {
                        // Mean reversion rally back to SMA170 (hits TP near SMA170)
                        let progress = (step - 14 + 1) as f64 / 9.0;
                        let c_val = p_base * (0.962 + progress * 0.055);
                        (o.max(c_val) + p_base * 0.004, o.min(c_val) - p_base * 0.002, c_val)
                    }
                    _ => {
                        // Neutral oscillation around SMA170
                        let c_val = p_base * (1.005 + ((step % 3) as f64 - 1.0) * 0.003);
                        (c_val + p_base * 0.002, c_val - p_base * 0.002, c_val)
                    }
                },
            };
            (o, h, l, c)
        };

        let safe_h = high.max(open).max(close);
        let safe_l = low.min(open).min(close).max(1.0);
        let volume = 150.0 + ((i * 17) % 100) as f64;

        bars.push(KlineBar {
            seq: 0,
            ts_open: ts,
            open,
            high: safe_h,
            low: safe_l,
            close,
            volume,
            amount: volume * close,
            pct_chg: Some((close - open) / open * 100.0),
            closed: true,
        });

        current_open = close;
    }

    bars
}
