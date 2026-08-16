use crate::data::base::KlineBar;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn timeframe_to_seconds(timeframe: &str) -> Option<u64> {
    let tf = timeframe.trim().to_lowercase();
    match tf.as_str() {
        "1m" => Some(60),
        "3m" => Some(3 * 60),
        "5m" => Some(5 * 60),
        "15m" => Some(15 * 60),
        "30m" => Some(30 * 60),
        "1h" => Some(3600),
        "2h" => Some(2 * 3600),
        "4h" => Some(4 * 3600),
        "6h" => Some(6 * 3600),
        "12h" => Some(12 * 3600),
        "1d" => Some(86400),
        "1w" => Some(7 * 86400),
        _ => {
            if let Some(num_str) = tf.strip_suffix('m') {
                num_str.parse::<u64>().ok().map(|n| n * 60)
            } else if let Some(num_str) = tf.strip_suffix('h') {
                num_str.parse::<u64>().ok().map(|n| n * 3600)
            } else if let Some(num_str) = tf.strip_suffix('d') {
                num_str.parse::<u64>().ok().map(|n| n * 86400)
            } else if let Some(num_str) = tf.strip_suffix('w') {
                num_str.parse::<u64>().ok().map(|n| n * 7 * 86400)
            } else {
                None
            }
        }
    }
}

pub fn current_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn seconds_until_bar_closes(ts_open_ms: i64, timeframe: &str, now_ms: Option<i64>) -> Option<u64> {
    let duration_s = timeframe_to_seconds(timeframe)?;
    let now = now_ms.unwrap_or_else(current_now_ms);
    let duration_ms = (duration_s as i64) * 1000;
    let elapsed_ms = now - ts_open_ms;
    if elapsed_ms == 0 {
        return Some(duration_s);
    }
    let remainder_ms = elapsed_ms.rem_euclid(duration_ms);
    if remainder_ms == 0 {
        return Some(if elapsed_ms > 0 { 0 } else { duration_s });
    }
    let remaining_ms = duration_ms - remainder_ms;
    Some(((remaining_ms as f64) / 1000.0).ceil() as u64)
}

pub fn is_bar_still_forming(bar: &KlineBar, timeframe: &str, now_ms: Option<i64>) -> bool {
    if bar.closed {
        return false;
    }
    let duration_s = match timeframe_to_seconds(timeframe) {
        Some(d) => d,
        None => return true,
    };
    let now = now_ms.unwrap_or_else(current_now_ms);
    let close_ms = bar.ts_open + (duration_s as i64) * 1000;
    now < close_ms
}

pub fn has_forming_bar_at_head(bars: &[KlineBar], timeframe: Option<&str>, now_ms: Option<i64>) -> bool {
    if bars.is_empty() {
        return false;
    }
    match timeframe {
        Some(tf) => is_bar_still_forming(&bars[0], tf, now_ms),
        None => !bars[0].closed,
    }
}
