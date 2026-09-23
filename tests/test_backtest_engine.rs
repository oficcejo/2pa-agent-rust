//! Comprehensive integration test suite for the backtesting engine and API endpoints.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use okx_2pa_agent::backtest::account::{PositionSide, VirtualAccount};
use okx_2pa_agent::backtest::cache::{CachedDecision, DecisionCache};
use okx_2pa_agent::backtest::data::{detect_gaps, generate_synthetic_candles};
use okx_2pa_agent::backtest::engine::BacktestEngine;
use okx_2pa_agent::backtest::matcher::{OrderMatcher, PendingOrder};
use okx_2pa_agent::backtest::types::{
    BacktestConfig, BacktestDataSource, BacktestTrade, EquityPoint,
};
use okx_2pa_agent::config::settings::Settings;
use okx_2pa_agent::data::base::KlineBar;
use okx_2pa_agent::data::snapshot::build_analysis_frame;
use okx_2pa_agent::web::server::create_router;
use okx_2pa_agent::web::service::WebTradingService;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const TEST_TOKEN: &str = "test-token-backtest-suite";

fn test_app() -> axum::Router {
    let settings = Settings {
        web_auth_token: TEST_TOKEN.to_string(),
        ..Default::default()
    };
    create_router(Arc::new(WebTradingService::new(settings)))
}

#[tokio::test]
async fn test_closed_htf_resampling_has_zero_lookahead_bias() {
    // Generate 16 fifteen-minute bars covering 4 hours (e.g. 08:00 to 12:00)
    let interval_15m = 900_000i64;
    let base_ts = 1710000000000i64 - (1710000000000i64 % 3_600_000i64); // align to clean hour (e.g. 08:00)
    let bars = generate_synthetic_candles(16, 60000.0, interval_15m, base_ts);

    // At bar 0 (08:00 - 08:15): The 08:00-09:00 hourly bar is NOT closed yet!
    let htf_0 = BacktestEngine::resample_closed_htf(&bars, "15m", "1H", 0);
    assert_eq!(htf_0.len(), 0, "Bar 0 cannot have any closed 1H bars");

    // At bar 1 (08:15 - 08:30): Still not closed
    let htf_1 = BacktestEngine::resample_closed_htf(&bars, "15m", "1H", 1);
    assert_eq!(htf_1.len(), 0);

    // At bar 3 (08:45 - 09:00): Exactly closes at 09:00! The first 1H bar (08:00 - 09:00) is now completed!
    let htf_3 = BacktestEngine::resample_closed_htf(&bars, "15m", "1H", 3);
    assert_eq!(htf_3.len(), 1, "Bar 3 completes the 08:00-09:00 1H bar");
    assert_eq!(htf_3[0].ts_open, base_ts);
    assert_eq!(htf_3[0].open, bars[0].open);
    assert_eq!(htf_3[0].close, bars[3].close);

    // At bar 4 (09:00 - 09:15): The latest closed 1H bar is STILL only the 08:00-09:00 bar!
    let htf_4 = BacktestEngine::resample_closed_htf(&bars, "15m", "1H", 4);
    assert_eq!(htf_4.len(), 1);

    // At bar 7 (09:45 - 10:00): The 09:00-10:00 bar now closes! Total 2 closed 1H bars
    let htf_7 = BacktestEngine::resample_closed_htf(&bars, "15m", "1H", 7);
    assert_eq!(htf_7.len(), 2);
    assert_eq!(htf_7[1].ts_open, base_ts + 3_600_000);
}

#[tokio::test]
async fn test_decision_cache_fingerprint_and_persistence() {
    let tmp_dir = std::env::temp_dir().join(format!("bt_cache_test_{}", uuid::Uuid::new_v4().simple()));
    let cache_file = tmp_dir.join("cache.json");

    let cache = DecisionCache::new(Some(cache_file.clone()));
    assert_eq!(cache.len(), 0);

    // Create a mock frame
    let bars = generate_synthetic_candles(250, 65000.0, 900_000, 1710000000000);
    let mut bars_desc = bars.clone();
    bars_desc.reverse();

    let frame = build_analysis_frame(&bars_desc, 50, "BTC-USDT-SWAP", "15m", None).unwrap();

    let fp1 = DecisionCache::compute_fingerprint("2pa_trend", &frame, None, None);
    let fp2 = DecisionCache::compute_fingerprint("2pa_trend", &frame, None, None);
    assert_eq!(fp1, fp2, "Identical frame states must produce identical fingerprints");

    let fp_dog = DecisionCache::compute_fingerprint("dog_reversion", &frame, None, None);
    assert_ne!(fp1, fp_dog, "Different strategies must produce different fingerprints");

    // Insert cached decision
    let cached = CachedDecision {
        strategy_id: "2pa_trend".to_string(),
        action: "OPEN".to_string(),
        order_type: "市价单".to_string(),
        order_direction: Some("做多".to_string()),
        entry_price: Some(65000.0),
        stop_loss_price: Some(64500.0),
        take_profit_price: Some(66000.0),
        trade_confidence: 88,
        reasoning: "Confirmed 2PA setup".to_string(),
        stage1_diagnosis: Some(json!({"eligible": true})),
        raw_decision: json!({"action": "OPEN"}),
        timestamp_ms: 1710000000000,
    };

    cache.insert(fp1.clone(), cached.clone());
    assert_eq!(cache.len(), 1);

    let retrieved = cache.get(&fp1).expect("Should retrieve cached decision");
    assert_eq!(retrieved.action, "OPEN");
    assert_eq!(retrieved.order_direction.as_deref(), Some("做多"));

    // Save and reload
    cache.save().expect("Save cache to disk");
    let cache_reloaded = DecisionCache::new(Some(cache_file.clone()));
    assert_eq!(cache_reloaded.len(), 1);
    assert!(cache_reloaded.get(&fp1).is_some());

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[tokio::test]
async fn test_virtual_account_risk_budgeting_and_pessimistic_fees() {
    let mut config = BacktestConfig::default();
    config.initial_capital = 10000.0;
    config.risk_percent = 1.0; // 1% = 100 USDT risk
    config.taker_fee_rate = 0.0005; // 0.05%
    config.slippage_rate = 0.0002; // 0.02%
    config.ct_val = 0.01; // 1 lot = 0.01 BTC
    config.lot_sz = 1.0;
    config.leverage = 5.0;

    let mut account = VirtualAccount::new(&config);
    assert_eq!(account.equity, 10000.0);
    assert_eq!(account.cash, 10000.0);

    // Entry 50000, Stop 49000 -> Diff = 1000 USDT per BTC -> 10 USDT risk per lot
    // 100 USDT risk budget -> ~10 lots
    let size = account.calculate_order_size(50000.0, 49000.0, 1.0, 50.0);
    assert!(size >= 8.0 && size <= 10.0, "Expected size around 9-10 lots, got {}", size);

    // Open Long position
    account
        .open_position(
            "2pa_trend",
            "SIG-1",
            "BTC-USDT-SWAP",
            PositionSide::Long,
            "市价单",
            50000.0,
            49000.0,
            52000.0,
            size,
            1710000000000,
        )
        .expect("Open position");

    assert!(account.position.is_some());
    let pos = account.position.as_ref().unwrap();
    // Pessimistic entry: buyer pays slippage (50000 * 1.0002 = 50010)
    assert!(pos.entry_price > 50000.0);
    assert!(account.total_fees_paid > 0.0);
    assert!(account.total_slippage_paid > 0.0);

    // Bar movement
    account.update_bar(51000.0, 49800.0, 50500.0);
    assert!(account.position.as_ref().unwrap().mfe_r > 0.0);

    // Close position at 52000 (Take Profit)
    let trade = account.close_position(52000.0, 1710000900000, "take_profit").expect("Close trade");
    assert_eq!(trade.exit_reason, "take_profit");
    assert!(trade.net_pnl > 0.0, "Expected profitable trade");
    assert!(trade.pnl_r > 0.0);
    assert!(account.position.is_none());
    assert!(account.equity > 10000.0, "Final equity should increase");

    // Exact mathematical match: total account equity gain must equal trade.net_pnl!
    let net_equity_gain = account.equity - account.initial_capital;
    assert!(
        (net_equity_gain - trade.net_pnl).abs() < 1e-6,
        "Account equity change ({}) must exactly equal trade net_pnl ({})",
        net_equity_gain,
        trade.net_pnl
    );
}

#[tokio::test]
async fn test_pessimistic_exit_priority_when_sl_and_tp_both_touched() {
    let config = BacktestConfig::default();
    let mut account = VirtualAccount::new(&config);

    // Open Long at 50000, SL at 49000, TP at 51000
    account
        .open_position(
            "2pa_trend",
            "SIG-PESSIMISTIC",
            "BTC-USDT-SWAP",
            PositionSide::Long,
            "市价单",
            50000.0,
            49000.0,
            51000.0,
            1.0,
            1710000000000,
        )
        .expect("Open");

    let matcher = OrderMatcher::new();

    // Wild bar: Low hits 48500 (SL touched!), High hits 51500 (TP touched!)
    let wild_bar = KlineBar {
        seq: 1,
        ts_open: 1710000900000,
        open: 50000.0,
        high: 51500.0,
        low: 48500.0,
        close: 50200.0,
        volume: 1000.0,
        amount: 50200000.0,
        pct_chg: None,
        closed: true,
    };

    let exit_result = matcher
        .check_mechanical_exit(&mut account, &wild_bar, 48)
        .expect("Exit check");

    assert!(exit_result.is_some());
    let trade = exit_result.unwrap();
    // Pessimistic branch: stop loss MUST win over take profit
    assert_eq!(trade.exit_reason, "stop_loss", "Pessimistic fill must trigger stop loss before target");
    assert!(trade.net_pnl < 0.0);
}

#[tokio::test]
async fn test_programmatic_gating_skips_chop_bars() {
    let mut config = BacktestConfig::default();
    config.allow_llm_calls = false; // deterministic diagnostic testing
    config.use_cache = false;

    let cache = DecisionCache::new(None);
    let engine = BacktestEngine::new(config, cache, None);

    // Generate 350 flat chop candles with almost zero volatility
    let mut bars = Vec::new();
    let mut ts = 1710000000000i64;
    for i in 0..350 {
        let p = 60000.0 + ((i % 2) as f64) * 2.0;
        bars.push(KlineBar {
            seq: 0,
            ts_open: ts,
            open: p,
            high: p + 1.0,
            low: p - 1.0,
            close: p,
            volume: 10.0,
            amount: 600000.0,
            pct_chg: None,
            closed: true,
        });
        ts += 900_000;
    }

    let report = engine.run("JOB-CHOP-TEST", &bars, None).await.expect("Run backtest on chop");

    // In a flat chop market, diagnostics must reject long and short, short-circuiting LLM calls!
    assert!(report.metrics.gated_bars_skipped > 100, "Chop market should gate >100 bars, got {}", report.metrics.gated_bars_skipped);
    assert_eq!(report.metrics.llm_evaluations, 0);
}

#[tokio::test]
async fn test_metrics_computations_on_known_trades() {
    let config = BacktestConfig::default();
    let account = VirtualAccount::new(&config);

    let trades = vec![
        BacktestTrade {
            trade_id: "T1".into(),
            signal_id: "S1".into(),
            strategy_id: "2pa_trend".into(),
            direction: "做多".into(),
            order_type: "市价单".into(),
            entry_time_ms: 100,
            entry_price: 100.0,
            exit_time_ms: 200,
            exit_price: 120.0,
            contracts: 1.0,
            notional_usdt: 120.0,
            stop_loss: 90.0,
            take_profit: 120.0,
            gross_pnl: 20.0,
            net_pnl: 18.0,
            pnl_percent: 18.0,
            pnl_r: 1.8,
            exit_reason: "take_profit".into(),
            fees: 2.0,
            slippage: 0.0,
            mfe_r: 2.0,
            mae_r: 0.0,
            hold_bars: 4,
            notes: "".into(),
        },
        BacktestTrade {
            trade_id: "T2".into(),
            signal_id: "S2".into(),
            strategy_id: "2pa_trend".into(),
            direction: "做空".into(),
            order_type: "市价单".into(),
            entry_time_ms: 300,
            entry_price: 120.0,
            exit_time_ms: 400,
            exit_price: 130.0,
            contracts: 1.0,
            notional_usdt: 130.0,
            stop_loss: 130.0,
            take_profit: 100.0,
            gross_pnl: -10.0,
            net_pnl: -11.0,
            pnl_percent: -11.0,
            pnl_r: -1.0,
            exit_reason: "stop_loss".into(),
            fees: 1.0,
            slippage: 0.0,
            mfe_r: 0.2,
            mae_r: 1.0,
            hold_bars: 2,
            notes: "".into(),
        },
    ];

    let equity_curve = vec![
        EquityPoint { timestamp_ms: 0, equity: 10000.0, cash: 10000.0, unrealized_pnl: 0.0, drawdown_pct: 0.0, in_position: false },
        EquityPoint { timestamp_ms: 200, equity: 10018.0, cash: 10018.0, unrealized_pnl: 0.0, drawdown_pct: 0.0, in_position: false },
        EquityPoint { timestamp_ms: 400, equity: 10007.0, cash: 10007.0, unrealized_pnl: 0.0, drawdown_pct: 0.11, in_position: false },
    ];

    let metrics = BacktestEngine::compute_metrics(&account, &trades, &equity_curve, 100, 90, 2, 0, "15m");

    assert_eq!(metrics.total_trades, 2);
    assert_eq!(metrics.winning_trades, 1);
    assert_eq!(metrics.losing_trades, 1);
    assert_eq!(metrics.win_rate, 50.0);
    // Profit factor = 18.0 / 11.0 ~= 1.636
    assert!((metrics.profit_factor - (18.0 / 11.0)).abs() < 1e-3);
    // Expectancy R = (1.8 - 1.0) / 2 = 0.4R
    assert!((metrics.expectancy_r - 0.4).abs() < 1e-3);
}

#[tokio::test]
async fn test_gap_detection() {
    let interval = 900_000i64; // 15m
    let mut bars = generate_synthetic_candles(10, 50000.0, interval, 1710000000000);

    // No gaps in continuous series
    let gaps_initial = detect_gaps(&bars, interval);
    assert_eq!(gaps_initial.len(), 0);

    // Inject a gap between bar 4 and 5 (skip 3 bars)
    bars[5].ts_open += interval * 3;
    for i in 6..bars.len() {
        bars[i].ts_open += interval * 3;
    }

    let gaps = detect_gaps(&bars, interval);
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].missing_bars, 3);
}

#[tokio::test]
async fn test_backtest_web_api_endpoints_end_to_end() {
    let router = test_app();

    // 1. Unauthenticated request to /api/backtest/run must be rejected with 401
    let unauth_req = Request::builder()
        .method("POST")
        .uri("/api/backtest/run")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({}).to_string()))
        .unwrap();

    let unauth_resp = router.clone().oneshot(unauth_req).await.unwrap();
    assert_eq!(unauth_resp.status(), StatusCode::UNAUTHORIZED);

    // 2. Submit valid synthetic backtest job
    let config = BacktestConfig {
        symbol: "BTC-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        data_source: BacktestDataSource::Synthetic,
        max_bars: 300,
        allow_llm_calls: false,
        ..Default::default()
    };

    let run_req = Request::builder()
        .method("POST")
        .uri("/api/backtest/run")
        .header(header::AUTHORIZATION, format!("Bearer {TEST_TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&config).unwrap()))
        .unwrap();

    let run_resp = router.clone().oneshot(run_req).await.unwrap();
    assert_eq!(run_resp.status(), StatusCode::ACCEPTED);

    let bytes = axum::body::to_bytes(run_resp.into_body(), 1 << 20).await.unwrap();
    let run_val: Value = serde_json::from_slice(&bytes).unwrap();
    let job_id = run_val["job_id"].as_str().expect("job_id in response").to_string();
    assert!(job_id.starts_with("BT-"));

    // 3. Poll status
    let mut completed = false;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let status_req = Request::builder()
            .method("GET")
            .uri(format!("/api/backtest/status/{}", job_id))
            .header(header::AUTHORIZATION, format!("Bearer {TEST_TOKEN}"))
            .body(Body::empty())
            .unwrap();

        let status_resp = router.clone().oneshot(status_req).await.unwrap();
        assert_eq!(status_resp.status(), StatusCode::OK);

        let sbytes = axum::body::to_bytes(status_resp.into_body(), 1 << 20).await.unwrap();
        let sval: Value = serde_json::from_slice(&sbytes).unwrap();
        if sval["status"] == "completed" {
            completed = true;
            break;
        }
    }
    assert!(completed, "Backtest job should complete within polling window");

    // 4. Fetch complete report
    let report_req = Request::builder()
        .method("GET")
        .uri(format!("/api/backtest/report/{}", job_id))
        .header(header::AUTHORIZATION, format!("Bearer {TEST_TOKEN}"))
        .body(Body::empty())
        .unwrap();

    let report_resp = router.clone().oneshot(report_req).await.unwrap();
    assert_eq!(report_resp.status(), StatusCode::OK);

    let rbytes = axum::body::to_bytes(report_resp.into_body(), 1 << 20).await.unwrap();
    let report_val: Value = serde_json::from_slice(&rbytes).unwrap();
    assert_eq!(report_val["job_id"], job_id);
    assert!(report_val["metrics"]["total_bars_processed"].as_u64().unwrap() > 0);
    assert!(report_val["equity_curve"].as_array().unwrap().len() > 10);
    assert!(
        report_val["metrics"]["total_trades"].as_u64().unwrap() > 0,
        "Web API backtest report must contain >0 trades, got 0"
    );
    assert!(
        !report_val["trades"].as_array().unwrap().is_empty(),
        "Web API backtest trades array must not be empty"
    );
}

#[tokio::test]
async fn test_stop_loss_priority_over_wick_and_actual_liquidation() {
    let mut config = BacktestConfig::default();
    config.initial_capital = 10000.0;
    config.leverage = 5.0;
    config.taker_fee_rate = 0.0005;
    config.slippage_rate = 0.0002;
    config.ct_val = 0.01;

    let mut account = VirtualAccount::new(&config);

    // 1. Position with tight Stop Loss at 49,000 (entry 50,000)
    account
        .open_position(
            "2pa_trend",
            "SIG-SL-PRIORITY",
            "BTC-USDT-SWAP",
            PositionSide::Long,
            "市价单",
            50000.0,
            49000.0,
            52000.0,
            10.0,
            1710000000000,
        )
        .expect("Open");

    let matcher = OrderMatcher::new();

    // Flash-crash wick candle: opens at 50,000, wicks all the way down to 10,000 (would liquidate 5x!), closes at 49,500
    let flash_wick_bar = KlineBar {
        seq: 1,
        ts_open: 1710000900000,
        open: 50000.0,
        high: 50100.0,
        low: 10000.0,
        close: 49500.0,
        volume: 5000.0,
        amount: 250000000.0,
        pct_chg: None,
        closed: true,
    };

    let exit_res = matcher
        .check_mechanical_exit(&mut account, &flash_wick_bar, 48)
        .expect("Exit check");

    assert!(exit_res.is_some(), "Position should have exited");
    let trade = exit_res.unwrap();
    // CRITICAL: Stop loss (at 49,000) was hit before price could reach the 10,000 wick!
    assert_eq!(trade.exit_reason, "stop_loss", "Stop loss must execute before deeper wick reaches liquidation");
    assert!(!account.is_liquidated, "Account must NOT be liquidated because stop loss protected it");
    assert!(trade.net_pnl < 0.0);

    // 2. Now test a trade that has NO stop loss (or SL set below liquidation)
    let mut config2 = config.clone();
    config2.initial_capital = 12000.0;
    let mut account2 = VirtualAccount::new(&config2);
    account2
        .open_position(
            "2pa_trend",
            "SIG-LIQ",
            "BTC-USDT-SWAP",
            PositionSide::Long,
            "市价单",
            50000.0,
            10000.0, // SL way below liquidation threshold
            60000.0,
            100.0, // 100 lots = 1.0 BTC = 50,000 USDT notional, locks 10,000 margin
            1710000000000,
        )
        .expect("Open without tight SL");

    let liq_px = account2.liquidation_price().expect("Must have liquidation price");
    assert!(liq_px > 30000.0, "Liquidation price should be above 30000 at 5x leverage (got {})", liq_px);

    let drop_bar = KlineBar {
        seq: 2,
        ts_open: 1710000900000,
        open: 50000.0,
        high: 50000.0,
        low: liq_px - 100.0, // breached liquidation threshold
        close: liq_px - 50.0,
        volume: 2000.0,
        amount: 80000000.0,
        pct_chg: None,
        closed: true,
    };

    let exit_res2 = matcher
        .check_mechanical_exit(&mut account2, &drop_bar, 48)
        .expect("Liquidation exit check");

    assert!(exit_res2.is_some());
    let trade2 = exit_res2.unwrap();
    assert_eq!(trade2.exit_reason, "liquidated");
    assert!(account2.is_liquidated, "Account should be marked liquidated");
    assert!(account2.equity >= 0.0, "Equity cannot be negative (insurance fund protects account)");
}

#[tokio::test]
async fn test_stop_loss_gap_slippage_fill() {
    let mut config = BacktestConfig::default();
    config.initial_capital = 10000.0;
    config.leverage = 2.0;

    let mut account = VirtualAccount::new(&config);

    // Open Long at 50,000, SL at 49,000
    account
        .open_position(
            "2pa_trend",
            "SIG-GAP-SL",
            "BTC-USDT-SWAP",
            PositionSide::Long,
            "市价单",
            50000.0,
            49000.0,
            55000.0,
            2.0,
            1710000000000,
        )
        .expect("Open");

    let matcher = OrderMatcher::new();

    // Weekend gap down candle: opens at 47,000 (well below SL of 49,000!)
    let gap_bar = KlineBar {
        seq: 1,
        ts_open: 1710000900000,
        open: 47000.0,
        high: 47200.0,
        low: 46800.0,
        close: 47100.0,
        volume: 100.0,
        amount: 4710000.0,
        pct_chg: None,
        closed: true,
    };

    let exit = matcher
        .check_mechanical_exit(&mut account, &gap_bar, 48)
        .expect("Exit check")
        .expect("Should trigger exit");

    assert_eq!(exit.exit_reason, "stop_loss");
    // Realistic gap execution: cannot exit at 49,000 when market opened at 47,000!
    // Slippage applies on 47,000
    assert!(exit.exit_price <= 47000.0, "Exit price must reflect gap open price ({}), got {}", 47000.0, exit.exit_price);
}

#[tokio::test]
async fn test_breakout_order_pessimistic_gap_fill() {
    let config = BacktestConfig::default();
    let mut account = VirtualAccount::new(&config);
    let mut matcher = OrderMatcher::new();

    // Long breakout order at 50,000
    matcher.add_pending_order(PendingOrder {
        signal_id: "SIG-BO-LONG".to_string(),
        strategy_id: "2pa_trend".to_string(),
        symbol: "BTC-USDT-SWAP".to_string(),
        side: PositionSide::Long,
        order_type: "突破单".to_string(),
        target_price: 50000.0,
        stop_loss: 49000.0,
        take_profit: 52000.0,
        contracts: 1.0,
        created_at_ms: 1710000000000,
        expiry_bars_remaining: 3,
    });

    // Bar gaps up over breakout price: opens at 51,000!
    let gap_up_bar = KlineBar {
        seq: 1,
        ts_open: 1710000900000,
        open: 51000.0,
        high: 51500.0,
        low: 50900.0,
        close: 51200.0,
        volume: 100.0,
        amount: 5120000.0,
        pct_chg: None,
        closed: true,
    };

    let filled = matcher.process_pending_orders(&mut account, &gap_up_bar);
    assert_eq!(filled.len(), 1);

    let pos = account.position.as_ref().unwrap();
    // Pessimistic fill: order fills at open (51,000) + slippage, NOT at 50,000!
    assert!(pos.entry_price >= 51000.0, "Breakout order in gap up must fill at or above open price");
}

#[tokio::test]
async fn test_backtest_start_end_time_bounding() {
    let interval = 900_000i64; // 15m
    let t0 = 1710000000000i64;
    // 400 bars total: 200 bars for indicator warmup, 101 bars for trading
    let bars = generate_synthetic_candles(400, 60000.0, interval, t0);

    let mut config = BacktestConfig::default();
    config.allow_llm_calls = false;
    config.use_cache = false;
    // Set start time at bar 200 and end time at bar 300
    config.start_time_ms = Some(t0 + 200 * interval);
    config.end_time_ms = Some(t0 + 300 * interval);

    let cache = DecisionCache::new(None);
    let engine = BacktestEngine::new(config, cache, None);

    let report = engine.run("JOB-TIME-BOUND", &bars, None).await.expect("Run time bounded");

    assert!(report.start_time_ms >= t0 + 200 * interval, "Report start time must be >= requested start_time_ms");
    assert!(report.end_time_ms <= t0 + 300 * interval, "Report end time must be <= requested end_time_ms");
    assert_eq!(report.metrics.total_bars_processed, 101);
}

#[tokio::test]
async fn test_synthetic_backtest_detects_setups_and_executes_trades() {
    let interval = 900_000i64; // 15m
    let t0 = 1710000000000i64;
    let bars = generate_synthetic_candles(500, 65000.0, interval, t0);

    let mut config = BacktestConfig::default();
    config.symbol = "BTC-USDT-SWAP".to_string();
    config.strategy_id = "2pa_trend".to_string();
    config.timeframe = "15m".to_string();
    config.allow_llm_calls = false;
    config.use_cache = false;
    config.max_bars = 500;

    let cache = DecisionCache::new(None);
    let engine = BacktestEngine::new(config, cache, None);

    let report = engine.run("JOB-SYNTH-2PA", &bars, None).await.expect("Run 2pa backtest");

    assert!(report.metrics.total_trades > 0, "Backtest must execute >0 trades, got 0");
    assert!(!report.trades.is_empty(), "Trades list must not be empty");
    for trade in &report.trades {
        assert!(trade.entry_price > 0.0);
        assert!(trade.exit_price > 0.0);
        assert!(trade.contracts > 0.0);
        assert!(!trade.exit_reason.is_empty());
    }
}

#[tokio::test]
async fn test_synthetic_backtest_all_strategies_execute_trades() {
    let interval = 900_000i64; // 15m
    let t0 = 1710000000000i64;
    for strat in ["2pa_trend", "dog_reversion", "dog_trend"] {
        let mut config = BacktestConfig::default();
        config.symbol = "BTC-USDT-SWAP".to_string();
        config.strategy_id = strat.to_string();
        config.timeframe = "15m".to_string();
        config.allow_llm_calls = false;
        config.use_cache = false;
        config.max_bars = 500;

        let cache = DecisionCache::new(None);
        let engine = BacktestEngine::new(config, cache, None);

        let bars = okx_2pa_agent::backtest::data::generate_synthetic_candles_for_strategy(strat, 500, 65000.0, interval, t0);
        let report = engine.run(&format!("JOB-{}", strat), &bars, None).await.expect("Run backtest");
        println!(
            "Strategy: {}, processed: {}, skipped: {}, trades: {}",
            strat, report.metrics.total_bars_processed, report.metrics.gated_bars_skipped, report.metrics.total_trades
        );
        assert!(report.metrics.total_trades > 0, "Strategy {} must execute >0 trades, got 0", strat);
    }
}


