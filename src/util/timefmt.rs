use chrono::{Local, TimeZone, Utc};

/// Return current local time timestamp in milliseconds.
pub fn now_local_ms() -> i64 {
    Local::now().timestamp_millis()
}

/// Return current UTC ISO 8601 string e.g. 2026-08-15T03:00:00.000Z
pub fn now_utc_iso() -> String {
    Utc::now().to_rfc3339()
}

/// Return current local time ISO 8601 string.
pub fn now_local_iso() -> String {
    Local::now().to_rfc3339()
}

/// Format a millisecond timestamp to local ISO string.
pub fn format_ms_to_local_iso(ts_ms: i64) -> String {
    match Local.timestamp_millis_opt(ts_ms) {
        chrono::LocalResult::Single(dt) => dt.to_rfc3339(),
        _ => Local::now().to_rfc3339(),
    }
}

/// Format millisecond timestamp for display e.g. "2026-08-15 11:00:00"
pub fn format_epoch_for_display(ts_ms: i64) -> String {
    match Local.timestamp_millis_opt(ts_ms) {
        chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        _ => ts_ms.to_string(),
    }
}
