use okx_2pa_agent::ai::json_validator::{extract_outer_json_object, parse_and_clean_json};
use okx_2pa_agent::config::settings::Settings;
use okx_2pa_agent::data::base::KlineBar;
use okx_2pa_agent::data::snapshot::build_analysis_frame;
use okx_2pa_agent::indicators::atr::{atr_full, state_after_atr};
use okx_2pa_agent::indicators::ema::{ema_full, state_after};
use okx_2pa_agent::indicators::sma::sma_full;
use okx_2pa_agent::web::server::create_router;
use okx_2pa_agent::web::service::WebTradingService;
use okx_2pa_agent::web::sessions::build_trading_session;
use std::sync::Arc;
use tower::ServiceExt;

fn mk_bar(seq: usize, ts_open: i64, close: f64) -> KlineBar {
    KlineBar {
        seq,
        ts_open,
        open: close,
        high: close * 1.01,
        low: close * 0.99,
        close,
        volume: 1000.0,
        amount: 0.0,
        pct_chg: None,
        closed: true,
    }
}

#[tokio::test]
async fn test_path_traversal_static_files() {
    let service = Arc::new(WebTradingService::new(Settings { web_auth_token: "test-password".into(), ..Default::default() }));
    let app = create_router(service);

    let req = axum::http::Request::builder()
        .uri("/static/%2e%2e%2fCargo.toml")
        .header("Authorization", "Bearer test-password")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    println!("encoded ../Cargo.toml -> {}", resp.status());
    assert_eq!(resp.status(), 404, "path traversal must be rejected");

    let service2 = Arc::new(WebTradingService::new(Settings { web_auth_token: "test-password".into(), ..Default::default() }));
    let app2 = create_router(service2);
    let req2 = axum::http::Request::builder()
        .uri("/static/%2e%2e%2f.env")
        .header("Authorization", "Bearer test-password")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp2 = app2.oneshot(req2).await.unwrap();
    println!("encoded ../.env -> {}", resp2.status());
    assert_eq!(resp2.status(), 404, "private files must not be served");

    let service3 = Arc::new(WebTradingService::new(Settings { web_auth_token: "test-password".into(), ..Default::default() }));
    let app3 = create_router(service3);
    let req3 = axum::http::Request::builder()
        .uri("/static/app.js")
        .header("Authorization", "Bearer test-password")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp3 = app3.oneshot(req3).await.unwrap();
    assert_eq!(resp3.status(), 200);
}

#[test]
fn test_warmup_insufficient_produces_missing_dev_pct() {
    let n_total = 175;
    let n_request = 50;
    let mut bars = Vec::new();
    let base_ts = 1_700_000_000_000i64;
    for i in 0..n_total {
        bars.push(mk_bar(i + 1, base_ts + i as i64 * 900_000, 100.0 + i as f64));
    }
    let frame = build_analysis_frame(&bars, n_request, "BTC-USDT", "15m", None)
        .expect("frame should build");
    let idx = 10;
    println!(
        "sma170[{}]={}, dev170_pct[{}]={}",
        idx, frame.indicators.sma170[idx], idx, frame.indicators.dev170_pct[idx]
    );
    assert!(
        frame.indicators.sma170[idx].is_nan(),
        "SMA170 is NaN without full warmup"
    );
    assert!(frame.indicators.dev170_pct[idx].is_nan(), "missing deviation must not be reported as zero");
}

#[test]
fn test_invalid_timezone_fails_closed() {
    let session = build_trading_session("custom", "Not/ARealZone", "09:00", "10:00", Some(&[0]));
    let monday_utc = chrono::DateTime::parse_from_rfc3339("2026-08-24T23:30:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let open = session.is_open_at(Some(monday_utc));
    println!("invalid tz, configured window 09:00-10:00, actual local ~23:30 -> open={}", open);
    assert!(!open, "invalid timezone must block trading");
}

#[test]
fn test_indicators_incremental_matches_full() {
    let mut seed = 42u64;
    let mut rand = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((seed >> 33) as f64) / (u32::MAX as f64)
    };
    let n = 400;
    let mut highs = Vec::with_capacity(n);
    let mut lows = Vec::with_capacity(n);
    let mut closes = Vec::with_capacity(n);
    let mut px = 100.0;
    for _ in 0..n {
        px += (rand() - 0.5) * 2.0;
        let h = px + rand();
        let l = px - rand();
        highs.push(h);
        lows.push(l);
        closes.push(px);
    }

    let ema_full_v = ema_full(&closes, 20);
    let ema_state = state_after(&closes, 20);
    assert!((ema_state.last - ema_full_v[n - 1]).abs() < 1e-9);

    let sma_full_v = sma_full(&closes, 14);
    let manual = closes[n - 14..].iter().sum::<f64>() / 14.0;
    assert!((sma_full_v[n - 1] - manual).abs() < 1e-9);

    let atr_full_v = atr_full(&highs, &lows, &closes, 14);
    let atr_state = state_after_atr(&highs, &lows, &closes, 14);
    assert!(
        (atr_state.last - atr_full_v[n - 1]).abs() < 1e-9,
        "atr incremental {} vs full {}",
        atr_state.last,
        atr_full_v[n - 1]
    );
}

#[test]
fn test_json_repair_preserves_string_content() {
    let raw = "{\"a\":\"p,}q\",\"b\":1,}";
    let extracted = extract_outer_json_object(raw);
    let parsed = parse_and_clean_json(&extracted, "stage2");
    let value = parsed.expect("trailing comma can be repaired");
    assert_eq!(value["a"], "p,}q", "string contents must survive repair");
}

#[test]
fn test_extract_outer_json_with_braces_in_string() {
    let raw = r#"prefix {"reasoning": "价格 {上涨} 了 \"快\"", "v": 1} suffix"#;
    let out = extract_outer_json_object(raw);
    let v: serde_json::Value = serde_json::from_str(&out).expect("should extract valid json");
    assert_eq!(v.get("v").and_then(|x| x.as_i64()), Some(1));
}
