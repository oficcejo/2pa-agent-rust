mod support;

use okx_2pa_agent::{
    config::settings::Settings,
    data::base::{KlineBar, KlineFrame},
    learning::{
        DailyDrawdownGuardHook, ExperienceWriter, HookAction, HookPipeline, HookRejection,
        OutcomeStore, PostOutcomeContext, PreAnalysisContext, PreExecutionContext,
        QualificationPolicy, Reconciler, ShadowTradingHook, Trade2Episode, TradingHook,
    },
    okx::trading::{AuditEntry, OKXTradeExecutor, BROKER_TAG, PA_CLIENT_ORDER_PREFIX},
    orchestrator::two_stage::TwoStageOrchestrator,
    records::{benchmark::load_benchmark_episodes, experience::ExperienceReader},
    strategies::{net_rr, validate_entry, Evidence, MIN_NET_RR, VERSION},
    web::positions::{
        execute_management, normalize_position, position_side, read_position, validate_stop_change,
    },
};
use rust_decimal::prelude::*;
use rust_decimal_macros::dec;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use support::{ok, routes, Mock};

fn temp_test_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("okx-e2e-{prefix}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn create_synthetic_bars(count: usize, base_price: f64) -> Vec<KlineBar> {
    let mut bars = Vec::with_capacity(count);
    let now = chrono::Utc::now().timestamp_millis();
    let step_ms = 900_000; // 15m
    let start_ts = now - (count as i64) * step_ms;

    for i in 0..count {
        let ts = start_ts + (i as i64) * step_ms;
        let p = base_price + (i as f64) * 0.5 + ((i % 5) as f64) * 0.2;
        bars.push(KlineBar {
            seq: i + 1,
            ts_open: ts,
            open: p - 0.2,
            high: p + 1.5,
            low: p - 1.2,
            close: p + 0.3,
            volume: 50.0 + (i as f64),
            amount: 0.0,
            pct_chg: None,
            closed: true,
        });
    }
    bars
}

// -----------------------------------------------------------------------------
// 1. Signal & Decision Generation
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_phase1_signal_and_decision_generation() {
    let root = temp_test_dir("phase1");
    let reply = json!({
        "cycle_position": "trend",
        "dominant_force": "bulls",
        "gate_result": "wait",
        "diagnosis_summary": "Testing 2PA analysis",
        "reasoning": "Waiting for confirmation",
        "decision": {
            "action": "WAIT",
            "order_type": "不下单",
            "order_direction": null,
            "entry_price": null,
            "stop_loss_price": null,
            "take_profit_price": null,
            "trade_confidence": 0,
            "estimated_win_rate": null,
            "traders_equation_passes": false,
            "reasoning": "Waiting for setup confirmation"
        },
        "terminal": { "outcome": "wait" }
    });
    let mock = Mock::start(std::collections::HashMap::from([(
        "/v1/chat/completions".into(),
        json!({
            "choices": [{"message": {"content": reply.to_string()}}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}
        }),
    )]))
    .await;

    let mut settings = Settings::default();
    settings.provider.base_url = mock.url.clone();
    settings.provider.api_key = "test".into();
    settings.validation.retry_max = 0;
    let orch = TwoStageOrchestrator::new(settings, root.join("records"));

    let bars = create_synthetic_bars(60, 2500.0);
    let frame = KlineFrame {
        symbol: "ETH-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        bars: bars.clone(),
        indicators: okx_2pa_agent::data::base::IndicatorBundle {
            ema20: vec![2500.0; 60],
            atr14: vec![10.0; 60],
            sma14: vec![2500.0; 60],
            sma170: vec![2500.0; 60],
            sma170_slope: vec![],
            dev170_pct: vec![],
        },
        snapshot_ts_local_ms: chrono::Utc::now().timestamp_millis(),
    };

    let record = orch
        .run_analysis_with_system(&frame, "2pa_trend")
        .await
        .expect("2PA analysis must succeed");

    assert_eq!(record.meta.trading_system, "2pa_trend");
    assert!(
        record
            .usage_total
            .get("total_tokens")
            .and_then(|v| v.as_u64())
            .unwrap()
            > 0
    );

    let s2_dec = record.stage2_decision.expect("stage2_decision present");
    let dec_inner = &s2_dec["decision"];
    assert_eq!(dec_inner["strategy_id"], "2pa_trend");
    assert!(["OPEN", "HOLD", "CLOSE_EARLY", "WAIT"].contains(&dec_inner["action"].as_str().unwrap()));
    assert!(dec_inner["trade_confidence"].as_u64().is_some());

    // 1.2 Stamping receipt linkage
    let mut decision_payload = dec_inner.clone();
    decision_payload["decision_record_id"] = json!(record.meta.record_id);
    decision_payload["prompt_version"] = json!(record.meta.prompt_version);
    decision_payload["prompt_hash"] = json!(record.meta.prompt_hash);

    assert_eq!(decision_payload["decision_record_id"], record.meta.record_id);
    assert_eq!(decision_payload["prompt_version"], record.meta.prompt_version);
}

// -----------------------------------------------------------------------------
// 2. Lifecycle Hook & Risk Verification
// -----------------------------------------------------------------------------
#[test]
fn test_phase2_lifecycle_hooks_and_risk_verification() {
    // 2.1 DailyDrawdownGuardHook: Circuit Breaker
    let guard = Arc::new(DailyDrawdownGuardHook::new(300.0));
    let mut pipeline = HookPipeline::new();
    pipeline.add_hook(guard.clone());

    let pre_anal = PreAnalysisContext {
        symbol: "BTC-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa_trend".to_string(),
        timestamp_ms: chrono::Utc::now().timestamp_millis(),
        account_equity_usd: Some(10000.0),
    };
    assert!(pipeline.run_pre_analysis(&pre_anal).is_ok());

    // Trip the breaker with loss of 350 USD (> 300.0)
    guard.record_loss(350.0);
    assert!(guard.is_tripped());
    assert_eq!(guard.current_drawdown_usd(), 350.0);

    // Pre-analysis must be halted
    let anal_err = pipeline.run_pre_analysis(&pre_anal).unwrap_err();
    assert!(matches!(anal_err, HookRejection::CircuitBreakerTriggered(_)));

    // Pre-execution must be halted
    let pre_exec = PreExecutionContext {
        symbol: "BTC-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa_trend".to_string(),
        decision: json!({"action": "OPEN", "order_type": "限价单", "entry_price": 50000.0}),
        account_balance_usd: 10000.0,
        is_shadow_mode: false,
    };
    let exec_err = pipeline.run_pre_execution(&pre_exec).unwrap_err();
    assert!(matches!(exec_err, HookRejection::CircuitBreakerTriggered(_)));

    // Reset clears the breaker
    guard.reset();
    assert!(!guard.is_tripped());
    assert_eq!(guard.current_drawdown_usd(), 0.0);
    assert!(pipeline.run_pre_analysis(&pre_anal).is_ok());

    // 2.2 ShadowTradingHook: Intercept and Virtual Matching
    let shadow = Arc::new(ShadowTradingHook::new());
    let mut shadow_pipeline = HookPipeline::new();
    shadow_pipeline.add_hook(shadow.clone());

    let shadow_exec_ctx = PreExecutionContext {
        symbol: "ETH-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa_trend".to_string(),
        decision: json!({
            "action": "OPEN",
            "order_direction": "做多",
            "entry_price": 2000.0,
            "stop_loss_price": 1960.0,
            "take_profit_price": 2080.0
        }),
        account_balance_usd: 5000.0,
        is_shadow_mode: true,
    };

    let hook_action = shadow_pipeline.run_pre_execution(&shadow_exec_ctx).expect("Pre-execution must succeed");
    match hook_action {
        HookAction::InterceptShadow { shadow_order_id, simulated_entry_price, .. } => {
            assert!(!shadow_order_id.is_empty());
            assert_eq!(simulated_entry_price, 2000.0);
        }
        _ => panic!("Expected HookAction::InterceptShadow"),
    }
    assert_eq!(shadow.active_positions().len(), 1);

    // Feed a bar that triggers take profit: high reaches 2090.0 >= 2080.0
    let bar_win = KlineBar {
        seq: 1,
        ts_open: 1700000000000,
        open: 2010.0,
        high: 2090.0,
        low: 1995.0,
        close: 2085.0,
        volume: 100.0,
        amount: 0.0,
        pct_chg: None,
        closed: true,
    };

    let resolved = shadow.match_bar("ETH-USDT-SWAP", &bar_win);
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].exit_reason, "take_profit");
    assert_eq!(resolved[0].r_multiple, 2.0); // (2080 - 2000) / 40 = 2.0
    assert!(resolved[0].mfe_r >= 2.0);
    assert_eq!(shadow.active_positions().len(), 0);
    assert_eq!(shadow.closed_outcomes().len(), 1);

    // 2.3 Program Enforcement: Risk-Reward >= 1.5 and ATR bounds
    // Net RR calculation: net_rr accounts for fees and slippage
    let symbol = "TEST-USDT-SWAP";
    let rr_pass = net_rr(symbol, 100.0, 98.0, 104.0);
    assert!(rr_pass >= MIN_NET_RR, "Expected net RR >= 1.5, got {}", rr_pass);

    let rr_fail = net_rr(symbol, 100.0, 98.0, 101.0);
    assert!(rr_fail < MIN_NET_RR, "Expected net RR < 1.5, got {}", rr_fail);

    // validate_entry enforces RR >= 1.5, stop in [0.8..3.0] ATR, and 0.3% min stop distance
    let evidence = Evidence {
        symbol: symbol.to_string(),
        timeframe: "15m".to_string(),
        strategy_id: "2pa_trend".to_string(),
        strategy_version: VERSION.to_string(),
        direction: "做多".to_string(),
        setup: "second_entry".to_string(),
        signal_ts_ms: 1000,
        atr: 2.0,
        reference_close: 100.0,
        invalidation: 98.0,
        target_bound: 106.0,
    };

    // Valid entry: stop distance = 2.0 (1.0 ATR), target = 105.0
    let valid_decision = json!({
        "order_direction": "做多",
        "entry_price": 100.0,
        "stop_loss_price": 97.5,
        "take_profit_price": 105.0,
    });
    assert!(validate_entry(symbol, &valid_decision, &evidence).is_ok());

    // Invalid entry 1: Stop distance too narrow (< 0.8 ATR = 1.6)
    let narrow_stop_decision = json!({
        "order_direction": "做多",
        "entry_price": 100.0,
        "stop_loss_price": 99.0, // distance = 1.0 < 0.8 * 2.0 = 1.6
        "take_profit_price": 105.0,
    });
    let err_narrow = validate_entry(symbol, &narrow_stop_decision, &evidence).unwrap_err();
    assert!(err_narrow.to_string().contains("波动率风险"));

    // Invalid entry 2: Stop distance too wide (> 3.0 ATR = 6.0)
    let wide_stop_decision = json!({
        "order_direction": "做多",
        "entry_price": 100.0,
        "stop_loss_price": 93.0, // distance = 7.0 > 6.0
        "take_profit_price": 115.0,
    });
    let err_wide = validate_entry(symbol, &wide_stop_decision, &evidence).unwrap_err();
    assert!(err_wide.to_string().contains("波动率风险"));

    // Invalid entry 3: RR < 1.5
    let low_rr_decision = json!({
        "order_direction": "做多",
        "entry_price": 100.0,
        "stop_loss_price": 97.5, // distance = 2.5
        "take_profit_price": 102.0, // gross reward = 2.0 -> RR < 1.0
    });
    let err_rr = validate_entry(symbol, &low_rr_decision, &evidence).unwrap_err();
    assert!(err_rr.to_string().contains("低于 1.5"));
}

// -----------------------------------------------------------------------------
// 3. Order Opening (开单)
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_phase3_order_opening_and_execution() {
    let mut resp = routes();
    // Add custom ticker and balance
    resp.insert("/api/v5/market/ticker?instId=TEST-USDT-SWAP".into(), ok(json!([{"last": "100.0"}])));
    resp.insert(
        "/api/v5/trade/order".into(),
        ok(json!([{"sCode": "0", "ordId": "ord_open_12345"}])),
    );
    let mock = Mock::start(resp).await;

    let root = temp_test_dir("phase3");
    let audit_file = root.join("trade_audit.jsonl");

    let executor = OKXTradeExecutor::new(
        mock.client.clone(),
        1.0,
        "cross",
        "net",
        3.0,
        true,
        40,
        120,
        3,
        Some(audit_file.clone()),
        true,
        2.0,
        25.0,
    );

    let decision = json!({
        "action": "OPEN",
        "order_type": "限价单",
        "order_direction": "做多",
        "entry_price": 100.0,
        "stop_loss_price": 98.0,
        "take_profit_price": 105.0,
        "trade_confidence": 80,
        "atr14": 2.0,
    });

    let signal_id = "sig_open_phase3_test";
    let (req, is_algo) = executor
        .build_request("TEST-USDT-SWAP", &decision, signal_id)
        .await
        .expect("Build request should succeed");

    assert!(!is_algo);
    // 3.1 Broker Tag verification
    assert_eq!(req["tag"], BROKER_TAG, "Broker Tag must be hardcoded");

    // 3.2 Client Order ID verification
    let cl_ord_id = req["clOrdId"].as_str().unwrap();
    assert!(cl_ord_id.starts_with(PA_CLIENT_ORDER_PREFIX));
    assert!(cl_ord_id.contains(signal_id));

    // Attached SL/TP orders
    let attached = &req["attachAlgoOrds"][0];
    let algo_cl_id = attached["attachAlgoClOrdId"].as_str().unwrap();
    assert!(algo_cl_id.starts_with(PA_CLIENT_ORDER_PREFIX));
    assert_eq!(attached["slTriggerPx"], "98.00");
    assert_eq!(attached["tpTriggerPx"], "105.00");

    // 3.3 Compute Order Size verification
    let computed_size = Decimal::from_str(req["sz"].as_str().unwrap()).unwrap();
    assert!(computed_size > Decimal::ZERO);
    // 2% of $1000 totalEq = $20 risk budget. Stop dist = $2. Unit risk = ~$2.1 -> ~9.5 contracts -> floor step
    assert!(computed_size <= dec!(10.0));

    // 3.4 Execution & Audit
    let bar_open_ts = chrono::Utc::now().timestamp_millis() - 900_000;
    let exec_res = executor
        .execute("TEST-USDT-SWAP", "15m", bar_open_ts, &decision)
        .await;

    assert!(exec_res.submitted, "Execution failed: {}", exec_res.reason);
    assert_eq!(exec_res.broker_tag, BROKER_TAG);
    assert!(exec_res.response.is_some());

    // Verify audit record was written
    let history = executor.audit_history(10);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].signal_id, exec_res.signal_id);
    assert!(history[0].submitted);
    assert_eq!(history[0].broker_tag, BROKER_TAG);

    // 3.5 Position Mutex: When position exists, block new entries
    let mut resp_blocked = routes();
    resp_blocked.insert(
        "/api/v5/account/positions".into(),
        ok(json!([{
            "instId": "TEST-USDT-SWAP",
            "pos": "2",
            "posSide": "net",
            "avgPx": "100.0",
            "markPx": "101.0",
            "mgnMode": "cross",
            "upl": "2.0",
            "uplRatio": "0.01",
            "lever": "3"
        }])),
    );
    let mock_blocked = Mock::start(resp_blocked).await;
    let exec_blocked = OKXTradeExecutor::new(
        mock_blocked.client.clone(),
        1.0,
        "cross",
        "net",
        3.0,
        true, // block_new_entries_when_position_open
        40,
        120,
        3,
        None,
        true,
        2.0,
        25.0,
    );

    let next_bar_ts = chrono::Utc::now().timestamp_millis() - 900_000;
    let blocked_res = exec_blocked
        .execute("TEST-USDT-SWAP", "15m", next_bar_ts, &decision)
        .await;
    assert!(!blocked_res.submitted);
    assert!(blocked_res.reason.contains("持仓互斥保护"));
}

// -----------------------------------------------------------------------------
// 4. Position Tracking & Closing (平仓)
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_phase4_position_tracking_and_closing() {
    let mut resp = routes();
    // Active long position
    let long_pos = json!({
        "instId": "TEST-USDT-SWAP",
        "pos": "5",
        "posSide": "net",
        "avgPx": "100.0",
        "markPx": "103.0",
        "mgnMode": "cross",
        "upl": "15.0",
        "uplRatio": "0.03",
        "lever": "3",
        "cTime": "1700000000000"
    });
    resp.insert("/api/v5/account/positions".into(), ok(json!([long_pos])));
    resp.insert(
        "/api/v5/trade/orders-algo-pending?ordType=conditional&instId=TEST-USDT-SWAP".into(),
        ok(json!([{
            "instId": "TEST-USDT-SWAP",
            "algoId": "algo_protect_999",
            "algoClOrdId": "pa_protect_999",
            "side": "sell",
            "posSide": "net",
            "slTriggerPx": "98.0",
            "tpTriggerPx": "106.0"
        }])),
    );
    resp.insert(
        "/api/v5/trade/amend-algos".into(),
        ok(json!([{"sCode": "0", "algoId": "algo_protect_999"}])),
    );
    resp.insert(
        "/api/v5/trade/close-position".into(),
        ok(json!([{"instId": "TEST-USDT-SWAP", "sCode": "0"}])),
    );

    let mock = Mock::start(resp).await;

    // 4.1 Position state normalization
    assert_eq!(position_side(&long_pos).unwrap(), "long");
    let norm = normalize_position(&long_pos).unwrap();
    assert_eq!(norm["direction"], "long");
    assert_eq!(norm["size"], 5.0);
    assert_eq!(norm["unrealized_pnl"], 15.0);

    let pos_ctx = read_position(&mock.client, "TEST-USDT-SWAP", "net")
        .await
        .expect("Read position should succeed");
    assert!(pos_ctx.has_position);
    assert_eq!(pos_ctx.pos_side, "long");
    assert_eq!(pos_ctx.pos_size, "5");
    assert_eq!(pos_ctx.open_avg_px, Some(100.0));
    assert_eq!(pos_ctx.mark_px, Some(103.0));
    assert_eq!(pos_ctx.current_sl, Some(98.0));
    assert_eq!(pos_ctx.current_tp, Some(106.0));
    assert_eq!(pos_ctx.algo_id, Some("algo_protect_999".to_string()));

    // 4.2 SL tightening validation
    // Tighten stop from 98.0 to 101.0 (below mark price 103.0): Valid!
    assert!(validate_stop_change(&pos_ctx, 101.0).is_ok());
    // Widen stop from 98.0 to 96.0: Rejected!
    assert!(validate_stop_change(&pos_ctx, 96.0).is_err());
    // Move stop above mark price 103.0: Rejected!
    assert!(validate_stop_change(&pos_ctx, 104.0).is_err());

    // Execute Move SL via execute_management
    let move_sl_decision = json!({
        "action": "MOVE_STOP_LOSS",
        "order_type": "修改止损",
        "new_stop_loss_price": 101.0,
    });
    let amend_res = execute_management(&mock.client, "TEST-USDT-SWAP", "net", &move_sl_decision).await;
    assert!(amend_res.is_ok(), "Amend SL failed: {:?}", amend_res);

    // 4.3 Manual Close Position (平仓)
    let close_decision = json!({
        "action": "CLOSE_EARLY",
        "order_type": "平仓",
    });
    let close_res = execute_management(&mock.client, "TEST-USDT-SWAP", "net", &close_decision).await;
    assert!(close_res.is_ok(), "Close position failed: {:?}", close_res);

    let calls = mock.calls.lock().unwrap();
    let close_call = calls.iter().find(|(m, p, _)| m == "POST" && p.starts_with("/api/v5/trade/close-position"));
    assert!(close_call.is_some());
    assert_eq!(close_call.unwrap().2["instId"], "TEST-USDT-SWAP");
    assert_eq!(close_call.unwrap().2["mgnMode"], "cross");
}

// -----------------------------------------------------------------------------
// 5. Post-Outcome Reconciliation (对账) & Trade2Episode Solidification
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_phase5_post_outcome_reconciliation_and_trade2episode() {
    let root = temp_test_dir("phase5");
    let store = OutcomeStore::new(root.join("outcomes"));
    let exp_writer = ExperienceWriter::new(root.join("experience"));
    let benchmark_dir = root.join("benchmark_episodes");

    let entry_ts = chrono::Utc::now().timestamp_millis() - 3_600_000;
    let signal_id = "sig_reconcile_test_888";

    // 5.1 Candle replay rows for walk-forward exit resolution
    // Bar 1: Entry bar, range 99.5 - 101.0
    // Bar 2: Moves up, touches target 104.0 (high=104.5) -> TakeProfit!
    let candle_rows = vec![
        json!([(entry_ts + 1_800_000).to_string(), "102.0", "104.5", "101.5", "104.0", "10", "0", "0", "1"]),
        json!([(entry_ts + 900_000).to_string(), "100.0", "102.5", "99.8", "102.0", "10", "0", "0", "1"]),
    ];

    let mut mock_routes = routes();
    mock_routes.insert(
        "/api/v5/trade/order".to_string(),
        ok(json!([{
            "state": "filled",
            "accFillSz": "1",
            "sz": "1",
            "avgPx": "100.0"
        }])),
    );
    mock_routes.insert(
        "/api/v5/market/candles".to_string(),
        ok(Value::Array(candle_rows)),
    );

    let mock = Mock::start(mock_routes).await;
    let reconciler = Reconciler::new(mock.client.clone(), store.clone(), exp_writer.clone());

    // AuditEntry with String prices and sizes to thoroughly verify string parsing fix!
    let audit = AuditEntry {
        strategy_id: "2pa_trend".into(),
        strategy_version: "v1".into(),
        decision_record_id: "rec_reconcile_888".into(),
        prompt_version: "v1".into(),
        prompt_hash: "hash_888".into(),
        cycle_position: "bullish_trend".into(),
        detected_patterns: vec!["supertrend_breakout".into()],
        id: "audit_888".into(),
        timestamp_ms: entry_ts,
        submitted: true,
        signal_id: signal_id.into(),
        instrument: "TEST-USDT-SWAP".into(),
        timeframe: "15m".into(),
        direction: "做多".into(),
        order_type: "限价单".into(),
        confidence: Some(json!(85)),
        size: Some(json!("1")),              // Value::String
        price: Some(json!("100.00")),        // Value::String
        stop_loss_price: Some(json!("98.00")), // Value::String
        take_profit_price: Some(json!("104.00")), // Value::String
        order_id: "ord_reconcile_888".into(),
        reason: String::new(),
        error_code: String::new(),
        broker_tag: BROKER_TAG.into(),
        deleted: false,
    };

    let policy = QualificationPolicy {
        require_filled: true,
        min_hold_bars: 1,
        max_abs_r: 25.0,
    };

    let report = reconciler.reconcile(&[audit], &policy, 96, 0, true).await;
    assert_eq!(report.checked, 1);
    assert_eq!(report.resolved, 1, "Reconciliation failed with errors: {:?}", report.errors);
    assert_eq!(report.experiences_written, 1);

    // 5.2 Verify Outcome calculation
    let outcome = store.load(signal_id).expect("Outcome must be saved");
    assert!(outcome.filled);
    assert_eq!(outcome.side, "long");
    assert_eq!(outcome.exit_reason, "take_profit");
    assert_eq!(outcome.exit_price, Some(104.0));
    // R-multiple: (104.0 - 100.0) / (100.0 - 98.0) = 4.0 / 2.0 = 2.0R
    assert!((outcome.r_multiple - 2.0).abs() < 1e-6);
    assert!(outcome.mfe_r >= 2.0);
    assert!(outcome.qualified);

    // 5.3 Experience Recording verification
    let exp_reader = ExperienceReader::new(root.join("experience"));
    let cases = exp_reader.read_for_stage2("bullish_trend", "long", &["supertrend_breakout".into()], 5);
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].case_type, "success");
    assert_eq!(cases[0].content["outcome"]["r_multiple"], 2.0);

    // 5.4 Trade2Episode Solidification (Data Flywheel)
    let klines = create_synthetic_bars(20, 95.0);
    let episode_path = Trade2Episode::solidify(&benchmark_dir, &outcome, klines, vec![])
        .expect("Solidify to episode must succeed");
    assert!(episode_path.exists());

    let episodes = load_benchmark_episodes(&benchmark_dir).expect("Load benchmark episodes should succeed");
    assert_eq!(episodes.len(), 1);
    let ep = &episodes[0];
    assert_eq!(ep.symbol, "TEST-USDT-SWAP");
    assert_eq!(ep.timeframe, "15m");
    assert_eq!(ep.expected_action, "OPEN_LONG");
    assert_eq!(ep.benchmark_r, 2.0);
    assert!(!ep.future_bars.is_empty(), "Auto-generated future bars must be populated");
}

// -----------------------------------------------------------------------------
// 6. Complete Integrated End-to-End Pipeline Test
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_full_integrated_lifecycle_end_to_end() {
    // Stitches all 5 phases into one coherent, flawless pipeline:
    // 1. Generate signal with 2PA / Dog ->
    // 2. Pass through Lifecycle Hooks & Risk Gates ->
    // 3. Open Order on Mock Venue with Broker Tag ->
    // 4. Normalize Position & Amend SL ->
    // 5. Walk-forward bar replay reconcile ->
    // 6. Record Experience & Solidify to Benchmark Episode
    let root = temp_test_dir("e2e_full");
    let audit_file = root.join("trade_audit.jsonl");
    let store = OutcomeStore::new(root.join("outcomes"));
    let exp_writer = ExperienceWriter::new(root.join("experience"));
    let benchmark_dir = root.join("benchmark_episodes");

    let entry_ts = chrono::Utc::now().timestamp_millis() - 900_000;
    let mut resp = routes();
    resp.insert(
        "/api/v5/trade/order".into(),
        ok(json!([{"sCode": "0", "ordId": "ord_e2e_complete"}])),
    );
    resp.insert(
        "/api/v5/trade/order?instId=TEST-USDT-SWAP&ordId=ord_e2e_complete".into(),
        ok(json!([{
            "state": "filled",
            "accFillSz": "1",
            "sz": "1",
            "avgPx": "100.0"
        }])),
    );
    resp.insert(
        "/api/v5/market/candles".into(),
        ok(json!([
            [(entry_ts + 1_800_000).to_string(), "102.0", "105.0", "101.0", "104.5", "10", "0", "0", "1"],
            [(entry_ts + 900_000).to_string(), "100.0", "102.0", "99.5", "102.0", "10", "0", "0", "1"],
        ])),
    );

    let mock = Mock::start(resp).await;

    // Step 1: Decision Generation
    let decision = json!({
        "action": "OPEN",
        "order_type": "限价单",
        "order_direction": "做多",
        "entry_price": 100.0,
        "stop_loss_price": 98.0,
        "take_profit_price": 104.0,
        "trade_confidence": 85,
        "decision_record_id": "rec_full_lifecycle",
        "prompt_version": "v1",
        "prompt_hash": "hash_v1",
        "cycle_position": "trend",
        "detected_patterns": ["second_entry"],
        "atr14": 2.0,
    });

    // Step 2: Lifecycle Hook & Circuit Breaker check
    let guard = Arc::new(DailyDrawdownGuardHook::new(500.0));
    let mut hook_pipeline = HookPipeline::new();
    hook_pipeline.add_hook(guard.clone());

    let pre_exec_ctx = PreExecutionContext {
        symbol: "TEST-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa_trend".to_string(),
        decision: decision.clone(),
        account_balance_usd: 1000.0,
        is_shadow_mode: false,
    };
    let hook_res = hook_pipeline.run_pre_execution(&pre_exec_ctx).unwrap();
    assert_eq!(hook_res, HookAction::Proceed);

    // Step 3: Order Execution on Venue
    let executor = OKXTradeExecutor::new(
        mock.client.clone(),
        1.0,
        "cross",
        "net",
        3.0,
        true,
        40,
        120,
        3,
        Some(audit_file.clone()),
        true,
        2.0,
        25.0,
    );

    let exec_res = executor
        .execute("TEST-USDT-SWAP", "15m", entry_ts, &decision)
        .await;
    assert!(exec_res.submitted, "Execution failed: {}", exec_res.reason);
    assert_eq!(exec_res.broker_tag, BROKER_TAG);

    // Step 4: Audit check
    let history = executor.audit_history(5);
    assert_eq!(history.len(), 1);
    let entry = &history[0];
    assert_eq!(entry.signal_id, exec_res.signal_id);

    // Step 5: Post-Outcome Reconciliation
    let reconciler = Reconciler::new(mock.client.clone(), store.clone(), exp_writer.clone());
    let policy = QualificationPolicy {
        require_filled: true,
        min_hold_bars: 1,
        max_abs_r: 25.0,
    };

    let report = reconciler.reconcile(&[entry.clone()], &policy, 96, 0, true).await;
    assert_eq!(report.resolved, 1);
    assert_eq!(report.experiences_written, 1);

    // Step 6: Verify Hook Post-Outcome feedback
    let outcome = store.load(&entry.signal_id).expect("Outcome exists");
    assert_eq!(outcome.exit_reason, "take_profit");
    assert_eq!(outcome.r_multiple, 2.0);

    hook_pipeline.run_post_outcome(&PostOutcomeContext { outcome: outcome.clone() }).unwrap();
    assert_eq!(guard.current_drawdown_usd(), 0.0, "Profitable trade does not increase drawdown");

    // Step 7: Trade2Episode Solidification
    let dummy_bars = create_synthetic_bars(10, 100.0);
    let ep_path = Trade2Episode::solidify(&benchmark_dir, &outcome, dummy_bars, vec![]).unwrap();
    assert!(ep_path.exists());

    let episodes = load_benchmark_episodes(&benchmark_dir).unwrap();
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].expected_action, "OPEN_LONG");
    assert_eq!(episodes[0].benchmark_r, 2.0);
}

// -----------------------------------------------------------------------------
// 7. Hedged Short Order Opening & Closing Lifecycle
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_hedged_short_order_opening_and_closing() {
    let mut resp = routes();
    resp.insert(
        "/api/v5/trade/order".into(),
        ok(json!([{"sCode": "0", "ordId": "ord_hedged_short"}])),
    );
    resp.insert(
        "/api/v5/account/positions".into(),
        ok(json!([{
            "instId": "TEST-USDT-SWAP",
            "pos": "3",
            "posSide": "short",
            "avgPx": "100.0",
            "markPx": "99.0",
            "mgnMode": "cross",
            "upl": "3.0",
            "uplRatio": "0.01",
            "lever": "3"
        }])),
    );
    resp.insert(
        "/api/v5/trade/close-position".into(),
        ok(json!([{"instId": "TEST-USDT-SWAP", "posSide": "short", "sCode": "0"}])),
    );

    let mock = Mock::start(resp).await;

    // Hedged mode executor
    let executor = OKXTradeExecutor::new(
        mock.client.clone(),
        1.0,
        "cross",
        "long_short", // Hedged mode!
        3.0,
        false,
        40,
        120,
        3,
        None,
        true,
        2.0,
        25.0,
    );

    let short_decision = json!({
        "action": "OPEN",
        "order_type": "限价单",
        "order_direction": "做空",
        "entry_price": 100.0,
        "stop_loss_price": 102.0,
        "take_profit_price": 95.0,
        "trade_confidence": 75,
        "atr14": 2.0,
    });

    let (req, _) = executor
        .build_request("TEST-USDT-SWAP", &short_decision, "sig_short_hedge")
        .await
        .unwrap();

    assert_eq!(req["side"], "sell");
    assert_eq!(req["posSide"], "short", "Hedged mode must include posSide: short");
    assert_eq!(req["tag"], BROKER_TAG);

    let bar_open_ts = chrono::Utc::now().timestamp_millis() - 900_000;
    let res = executor
        .execute("TEST-USDT-SWAP", "15m", bar_open_ts, &short_decision)
        .await;
    assert!(res.submitted);

    // Read normalized position in long_short mode
    let pos = read_position(&mock.client, "TEST-USDT-SWAP", "long_short")
        .await
        .unwrap();
    assert!(pos.has_position);
    assert_eq!(pos.pos_side, "short");

    // Close hedged short position
    let close_decision = json!({
        "action": "CLOSE_EARLY",
        "order_type": "平仓",
    });
    let close_res = execute_management(&mock.client, "TEST-USDT-SWAP", "long_short", &close_decision)
        .await
        .unwrap();
    assert_eq!(close_res["instId"], "TEST-USDT-SWAP");

    let calls = mock.calls.lock().unwrap();
    let close_call = calls.iter().find(|(m, p, _)| m == "POST" && p.starts_with("/api/v5/trade/close-position"));
    assert!(close_call.is_some());
    assert_eq!(close_call.unwrap().2["posSide"], "short", "Close position payload must pass posSide: short in hedged mode");
}

// -----------------------------------------------------------------------------
// 8. Order Expiry & Cancellation Lifecycle
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_order_cancellation_and_expiry_lifecycle() {
    let old_ts = (chrono::Utc::now().timestamp_millis() - 4_000_000).to_string();
    let fresh_ts = chrono::Utc::now().timestamp_millis().to_string();

    let mut resp = routes();
    resp.insert(
        "/api/v5/trade/orders-pending".into(),
        ok(json!([
            {"ordId": "ord_old_owned", "clOrdId": "pa_old_1", "cTime": old_ts, "tag": BROKER_TAG},
            {"ordId": "ord_fresh_owned", "clOrdId": "pa_fresh_1", "cTime": fresh_ts, "tag": BROKER_TAG},
            {"ordId": "ord_manual", "clOrdId": "manual_1", "cTime": old_ts}
        ])),
    );
    resp.insert(
        "/api/v5/trade/orders-algo-pending".into(),
        ok(json!([
            {"algoId": "algo_old_trigger", "algoClOrdId": "pa_old_trig", "cTime": old_ts, "tag": BROKER_TAG}
        ])),
    );
    resp.insert(
        "/api/v5/trade/cancel-order".into(),
        ok(json!([{"sCode": "0"}])),
    );
    resp.insert(
        "/api/v5/trade/cancel-algos".into(),
        ok(json!([{"sCode": "0"}])),
    );

    let mock = Mock::start(resp).await;
    let executor = OKXTradeExecutor::new(
        mock.client.clone(),
        1.0,
        "cross",
        "net",
        3.0,
        true,
        40,
        120,
        3,
        None,
        true,
        2.0,
        25.0,
    );

    let cancelled = executor
        .cancel_expired_entries("TEST-USDT-SWAP", "15m")
        .await
        .unwrap();

    // Must only cancel the 2 old owned orders (ord_old_owned and algo_old_trigger), NEVER ord_manual or ord_fresh_owned
    assert_eq!(cancelled, 2);

    let calls = mock.calls.lock().unwrap();
    let cancels: Vec<_> = calls.iter().filter(|(m, p, _)| m == "POST" && p.contains("cancel")).collect();
    assert_eq!(cancels.len(), 2);
    assert_eq!(cancels[0].2["ordId"], "ord_old_owned");
    assert_eq!(cancels[1].2[0]["algoId"], "algo_old_trigger");
}

// -----------------------------------------------------------------------------
// 9. Losing Trade Pessimistic Reconciliation & Drawdown Guard Feedback
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_losing_trade_pessimistic_reconciliation_and_drawdown_flywheel() {
    let root = temp_test_dir("losing_trade");
    let store = OutcomeStore::new(root.join("outcomes"));
    let exp_writer = ExperienceWriter::new(root.join("experience"));
    let benchmark_dir = root.join("benchmark_episodes");

    let entry_ts = chrono::Utc::now().timestamp_millis() - 1_800_000;
    let signal_id = "sig_loss_test_456";

    // Replay bar touches BOTH stop loss (low=97.0 <= 98.0) AND take profit (high=105.0 >= 104.0)
    // Pessimistic rule MUST trigger stop loss first!
    let candle_rows = vec![
        json!([(entry_ts + 900_000).to_string(), "100.0", "105.0", "97.0", "98.0", "10", "0", "0", "1"]),
    ];

    let mut mock_routes = routes();
    mock_routes.insert(
        "/api/v5/trade/order".to_string(),
        ok(json!([{
            "state": "filled",
            "accFillSz": "2",
            "sz": "2",
            "avgPx": "100.0"
        }])),
    );
    mock_routes.insert(
        "/api/v5/market/candles".to_string(),
        ok(Value::Array(candle_rows)),
    );

    let mock = Mock::start(mock_routes).await;
    let reconciler = Reconciler::new(mock.client.clone(), store.clone(), exp_writer.clone());

    let audit = AuditEntry {
        strategy_id: "2pa_trend".into(),
        strategy_version: "2026-09-v1".into(),
        decision_record_id: "rec_loss_456".into(),
        prompt_version: "v1".into(),
        prompt_hash: "hash_v1".into(),
        cycle_position: "breakout_pullback".into(),
        detected_patterns: vec!["fakeout".into()],
        id: "audit_loss_456".into(),
        timestamp_ms: entry_ts,
        submitted: true,
        signal_id: signal_id.into(),
        instrument: "TEST-USDT-SWAP".into(),
        timeframe: "15m".into(),
        direction: "做多".into(),
        order_type: "限价单".into(),
        confidence: Some(json!(70)),
        size: Some(json!("2")),
        price: Some(json!("100.0")),
        stop_loss_price: Some(json!("98.0")),
        take_profit_price: Some(json!("104.0")),
        order_id: "ord_loss_456".into(),
        reason: String::new(),
        error_code: String::new(),
        broker_tag: BROKER_TAG.into(),
        deleted: false,
    };

    let policy = QualificationPolicy {
        require_filled: true,
        min_hold_bars: 1,
        max_abs_r: 25.0,
    };

    let report = reconciler.reconcile(&[audit], &policy, 96, 0, true).await;
    assert_eq!(report.resolved, 1);
    assert_eq!(report.experiences_written, 1);

    let outcome = store.load(signal_id).expect("Outcome exists");
    // Pessimistic stop loss execution!
    assert_eq!(outcome.exit_reason, "stop_loss");
    assert_eq!(outcome.exit_price, Some(98.0));
    assert_eq!(outcome.r_multiple, -1.0); // (98 - 100) / 2 = -1.0R
    assert!(outcome.realized_pnl_usd < 0.0);

    // Experience reader should retrieve it as a failure case
    let exp_reader = ExperienceReader::new(root.join("experience"));
    let cases = exp_reader.read_for_stage2("breakout_pullback", "long", &["fakeout".into()], 5);
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].case_type, "failure");
    assert_eq!(cases[0].content["outcome"]["exit_reason"], "stop_loss");

    // Feed outcome to DailyDrawdownGuardHook
    let guard = DailyDrawdownGuardHook::new(50.0);
    assert_eq!(guard.current_drawdown_usd(), 0.0);
    guard.post_outcome(&PostOutcomeContext { outcome: outcome.clone() }).unwrap();
    // Realized pnl was -4.0 USD (loss of 4.0 USD)
    assert!(guard.current_drawdown_usd() > 0.0);

    // Solidify losing trade to benchmark episode: must have expected_action = "WAIT"
    let dummy_bars = create_synthetic_bars(10, 100.0);
    Trade2Episode::solidify(&benchmark_dir, &outcome, dummy_bars, vec![]).unwrap();
    let episodes = load_benchmark_episodes(&benchmark_dir).unwrap();
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].expected_action, "WAIT", "Losing trade benchmark episode must specify WAIT action");
    assert_eq!(episodes[0].benchmark_r, 0.0);
}

// -----------------------------------------------------------------------------
// 10. Market and Breakout (Trigger) Order Request Construction
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_market_and_trigger_breakout_order_construction() {
    let mut resp = routes();
    resp.insert("/api/v5/market/ticker?instId=TEST-USDT-SWAP".into(), ok(json!([{"last": "100.5"}])));
    let mock = Mock::start(resp).await;

    let executor = OKXTradeExecutor::new(
        mock.client.clone(),
        1.0,
        "cross",
        "net",
        3.0,
        true,
        40,
        120,
        3,
        None,
        true,
        2.0,
        25.0,
    );

    // 10.1 Market Order
    let market_decision = json!({
        "action": "OPEN",
        "order_type": "市价单",
        "order_direction": "做多",
        "entry_price": 100.0,
        "stop_loss_price": 98.0,
        "take_profit_price": 105.0,
        "trade_confidence": 80,
    });
    let (mkt_req, is_algo) = executor
        .build_request("TEST-USDT-SWAP", &market_decision, "sig_mkt")
        .await
        .unwrap();
    assert!(!is_algo);
    assert_eq!(mkt_req["ordType"], "market");
    assert!(mkt_req.get("px").is_none(), "Market order must not specify fixed px");
    assert_eq!(mkt_req["tag"], BROKER_TAG);

    // 10.2 Breakout / Trigger Order
    let breakout_decision = json!({
        "action": "OPEN",
        "order_type": "突破单",
        "order_direction": "做多",
        "entry_price": 101.0,
        "stop_loss_price": 99.0,
        "take_profit_price": 106.0,
        "trade_confidence": 80,
    });
    let (trig_req, is_algo_trig) = executor
        .build_request("TEST-USDT-SWAP", &breakout_decision, "sig_trig")
        .await
        .unwrap();
    assert!(is_algo_trig);
    assert_eq!(trig_req["ordType"], "trigger");
    assert_eq!(trig_req["triggerPx"], "101.00");
    assert_eq!(trig_req["orderPx"], "-1"); // Market execution upon trigger
    assert!(trig_req["algoClOrdId"].as_str().unwrap().starts_with(PA_CLIENT_ORDER_PREFIX));
    assert_eq!(trig_req["tag"], BROKER_TAG);
}

// -----------------------------------------------------------------------------
// 11. Partial Fill Followed by Cancellation & Scaled Outcome Verification
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_partial_fill_cancellation_and_scaled_outcome() {
    let root = temp_test_dir("partial_cancel");
    let store = OutcomeStore::new(root.join("outcomes"));
    let exp_writer = ExperienceWriter::new(root.join("experience"));

    let entry_ts = chrono::Utc::now().timestamp_millis() - 1_800_000;
    let signal_id = "sig_partial_cancel_999";

    // Replay bar reaches take profit: 104.0
    let candle_rows = vec![
        json!([(entry_ts + 900_000).to_string(), "100.0", "104.5", "99.5", "104.0", "10", "0", "0", "1"]),
    ];

    let mut mock_routes = routes();
    // Order was placed for sz = "2", but only filled 0.5 contracts before the remainder was canceled!
    mock_routes.insert(
        "/api/v5/trade/order".to_string(),
        ok(json!([{
            "state": "canceled",
            "accFillSz": "0.5",
            "sz": "2",
            "avgPx": "100.0"
        }])),
    );
    mock_routes.insert(
        "/api/v5/market/candles".to_string(),
        ok(Value::Array(candle_rows)),
    );

    let mock = Mock::start(mock_routes).await;
    let reconciler = Reconciler::new(mock.client.clone(), store.clone(), exp_writer.clone());

    let audit = AuditEntry {
        strategy_id: "2pa_trend".into(),
        strategy_version: "2026-09-v1".into(),
        decision_record_id: "rec_part_cancel_999".into(),
        prompt_version: "v1".into(),
        prompt_hash: "hash_v1".into(),
        cycle_position: "breakout_pullback".into(),
        detected_patterns: vec!["pullback_entry".into()],
        id: "audit_part_cancel_999".into(),
        timestamp_ms: entry_ts,
        submitted: true,
        signal_id: signal_id.into(),
        instrument: "TEST-USDT-SWAP".into(),
        timeframe: "15m".into(),
        direction: "做多".into(),
        order_type: "限价单".into(),
        confidence: Some(json!(80)),
        size: Some(json!("2")),                // Requested size was 2
        price: Some(json!("100.0")),
        stop_loss_price: Some(json!("98.0")),
        take_profit_price: Some(json!("104.0")),
        order_id: "ord_part_cancel_999".into(),
        reason: String::new(),
        error_code: String::new(),
        broker_tag: BROKER_TAG.into(),
        deleted: false,
    };

    let policy = QualificationPolicy {
        require_filled: true,
        min_hold_bars: 1,
        max_abs_r: 25.0,
    };

    let report = reconciler.reconcile(&[audit], &policy, 96, 0, true).await;
    assert_eq!(report.checked, 1);
    assert_eq!(report.resolved, 1, "Partial fill before cancel MUST resolve as filled trade");
    assert_eq!(report.unfilled, 0, "Partial fill MUST NOT be misclassified as unfilled");
    assert_eq!(report.experiences_written, 1);

    let outcome = store.load(signal_id).expect("Outcome must exist");
    assert!(outcome.filled, "Order with accFillSz > 0 must be marked filled");
    assert_eq!(outcome.fill_ratio, 0.25, "0.5 / 2.0 = 0.25 fill ratio");
    assert_eq!(outcome.size, 0.5, "Outcome size must scale to filled quantity (0.5), not requested (2.0)");
    assert_eq!(outcome.exit_reason, "take_profit");
    assert_eq!(outcome.exit_price, Some(104.0));
    assert_eq!(outcome.r_multiple, 2.0); // (104 - 100) / 2 = 2.0R

    // PnL check: (104.0 - 100.0) * 1.0 * 0.5 = 2.0 USD (accurately based on 0.5 filled, NOT 8.0 USD!)
    assert!((outcome.realized_pnl_usd - 2.0).abs() < 1e-4, "Realized PnL must be $2.0, got ${}", outcome.realized_pnl_usd);
}

// -----------------------------------------------------------------------------
// 12. Breakout (Trigger) Order Full Execution & Reconciliation
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_breakout_trigger_order_execution_and_reconciliation() {
    let root = temp_test_dir("breakout_exec");
    let audit_file = root.join("trade_audit.jsonl");
    let store = OutcomeStore::new(root.join("outcomes"));
    let exp_writer = ExperienceWriter::new(root.join("experience"));
    let benchmark_dir = root.join("benchmark_episodes");

    let entry_ts = chrono::Utc::now().timestamp_millis() - 900_000;
    let mut resp = routes();
    // Algo order placement response
    resp.insert(
        "/api/v5/trade/order-algo".into(),
        ok(json!([{"sCode": "0", "algoId": "algo_trigger_777"}])),
    );
    // Algo order query during reconciliation: triggered and filled!
    resp.insert(
        "/api/v5/trade/order-algo?algoId=algo_trigger_777".into(),
        ok(json!([{
            "instId": "TEST-USDT-SWAP",
            "algoId": "algo_trigger_777",
            "state": "effective",
            "actualPx": "101.0",
            "actualSz": "1",
            "sz": "1",
            "triggerPx": "101.0"
        }])),
    );
    // Candle replay hits take profit at 105.0
    resp.insert(
        "/api/v5/market/candles".into(),
        ok(json!([
            [(entry_ts + 1_800_000).to_string(), "101.0", "105.5", "100.5", "105.0", "10", "0", "0", "1"]
        ])),
    );

    let mock = Mock::start(resp).await;
    let executor = OKXTradeExecutor::new(
        mock.client.clone(),
        1.0,
        "cross",
        "net",
        3.0,
        true,
        40,
        120,
        3,
        Some(audit_file.clone()),
        true,
        2.0,
        25.0,
    );

    let breakout_decision = json!({
        "action": "OPEN",
        "order_type": "突破单",
        "order_direction": "做多",
        "entry_price": 101.0,
        "stop_loss_price": 99.0,
        "take_profit_price": 105.0,
        "trade_confidence": 85,
        "cycle_position": "consolidation_breakout",
        "detected_patterns": ["range_breakout"],
        "atr14": 2.0,
    });

    let exec_res = executor
        .execute("TEST-USDT-SWAP", "15m", entry_ts, &breakout_decision)
        .await;
    assert!(exec_res.submitted, "Breakout order execution failed: {}", exec_res.reason);
    assert_eq!(exec_res.broker_tag, BROKER_TAG);

    let history = executor.audit_history(5);
    assert_eq!(history.len(), 1);
    let audit_row = &history[0];
    assert!(["trigger", "突破单"].contains(&audit_row.order_type.as_str()), "Audit order_type must be trigger or 突破单, got {}", audit_row.order_type);
    assert_eq!(audit_row.order_id, "algo_trigger_777");

    // Reconcile the breakout order!
    let reconciler = Reconciler::new(mock.client.clone(), store.clone(), exp_writer.clone());
    let policy = QualificationPolicy {
        require_filled: true,
        min_hold_bars: 1,
        max_abs_r: 25.0,
    };

    let report = reconciler.reconcile(&[audit_row.clone()], &policy, 96, 0, true).await;
    assert_eq!(report.resolved, 1, "Breakout order reconciliation failed: {:?}", report.errors);
    assert_eq!(report.experiences_written, 1);

    let outcome = store.load(&audit_row.signal_id).expect("Outcome exists");
    assert!(outcome.filled);
    assert_eq!(outcome.exit_reason, "take_profit");
    assert_eq!(outcome.exit_price, Some(105.0));
    assert_eq!(outcome.r_multiple, 2.0); // (105 - 101) / 2 = 2.0R

    // Solidify to benchmark episode
    let dummy_bars = create_synthetic_bars(10, 101.0);
    Trade2Episode::solidify(&benchmark_dir, &outcome, dummy_bars, vec![]).unwrap();
    let episodes = load_benchmark_episodes(&benchmark_dir).unwrap();
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].expected_action, "OPEN_LONG");
    assert_eq!(episodes[0].benchmark_r, 2.0);
}

// -----------------------------------------------------------------------------
// 13. Trading Session Boundaries & Execution Gating
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_trading_session_boundaries_and_execution_gating() {
    use okx_2pa_agent::web::sessions::build_trading_session;

    // Build Asia session: 09:00 - 16:00 (Asia/Shanghai)
    let session = build_trading_session("asia", "Asia/Shanghai", "09:00", "16:00", Some(&[0, 1, 2, 3, 4]));

    // 1. Morning before market open: Friday 08:30 Beijing time (UTC 00:30)
    let pre_market = chrono::DateTime::parse_from_rfc3339("2026-09-18T08:30:00+08:00")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert!(!session.is_open_at(Some(pre_market)), "08:30 Beijing time must be outside Asia session");

    // 2. Active trading hours: Friday 10:30 Beijing time (UTC 02:30)
    let in_session = chrono::DateTime::parse_from_rfc3339("2026-09-18T10:30:00+08:00")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert!(session.is_open_at(Some(in_session)), "10:30 Beijing time must be inside Asia session");

    // 3. Post-market close: Friday 17:00 Beijing time (UTC 09:00)
    let post_market = chrono::DateTime::parse_from_rfc3339("2026-09-18T17:00:00+08:00")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert!(!session.is_open_at(Some(post_market)), "17:00 Beijing time must be outside Asia session");

    // 4. Weekend: Saturday 11:00 Beijing time
    let weekend = chrono::DateTime::parse_from_rfc3339("2026-09-19T11:00:00+08:00")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert!(!session.is_open_at(Some(weekend)), "Saturday must be closed for weekday sessions");
}

