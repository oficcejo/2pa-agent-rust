use okx_2pa_agent::data::base::KlineBar;
use okx_2pa_agent::data::base::KlineFrame;
use okx_2pa_agent::data::geometry::compute_kline_geometry_features;
use okx_2pa_agent::data::snapshot::compute_indicators;
use okx_2pa_agent::indicators::atr::atr_full;
use okx_2pa_agent::indicators::ema::ema_full;

#[test]
fn test_ema_calculation() {
    let prices = vec![10.0, 11.0, 12.0, 13.0, 14.0];
    let ema = ema_full(&prices, 3);
    assert!(ema[0].is_nan());
    assert!(ema[1].is_nan());
    assert!((ema[2] - 11.0).abs() < 1e-6); // mean of 10, 11, 12
    assert!((ema[3] - 12.0).abs() < 1e-6); // 13 * 0.5 + 11.0 * 0.5
    assert!((ema[4] - 13.0).abs() < 1e-6); // 14 * 0.5 + 12.0 * 0.5
}

#[test]
fn test_atr_calculation() {
    let highs = vec![10.0, 11.0, 12.0, 13.0];
    let lows = vec![9.0, 10.0, 11.0, 12.0];
    let closes = vec![9.5, 10.5, 11.5, 12.5];
    let atr = atr_full(&highs, &lows, &closes, 2);
    assert!(atr[0].is_nan());
    assert!(!atr[1].is_nan());
    assert!((atr[1] - 1.25).abs() < 1e-6);
}

#[test]
fn test_geometry_features() {
    let mut bars = Vec::new();
    for i in 1..=5 {
        bars.push(KlineBar {
            seq: i,
            ts_open: (1000 - i * 60) as i64,
            open: 100.0 + (i as f64),
            high: 105.0 + (i as f64),
            low: 95.0 + (i as f64),
            close: 104.0 + (i as f64),
            volume: 100.0,
            amount: 0.0,
            pct_chg: None,
            closed: true,
        });
    }

    let indicators = compute_indicators(&bars);
    let frame = KlineFrame {
        symbol: "BTC-USDT".to_string(),
        timeframe: "15m".to_string(),
        bars,
        indicators,
        snapshot_ts_local_ms: 1000,
    };

    let features = compute_kline_geometry_features(&frame, Some(5));
    assert_eq!(features.len(), 5);
    assert_eq!(features[0].seq, 1);
}
