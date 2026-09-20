use okx_2pa_agent::data::base::KlineBar;
use okx_2pa_agent::learning::{
    compute_multidimensional_metrics, group_by_strategy, group_by_symbol, DailyDrawdownGuardHook,
    HookAction, HookPipeline, HookRejection, PreAnalysisContext, PreExecutionContext,
    ShadowTradingHook, TradeOutcome,
};
use std::sync::Arc;

fn sample_outcome(strategy: &str, symbol: &str, r: f64, pnl_usd: f64) -> TradeOutcome {
    TradeOutcome {
        signal_id: uuid::Uuid::new_v4().simple().to_string(),
        decision_record_id: "rec_1".to_string(),
        strategy_id: strategy.to_string(),
        strategy_version: "v1".to_string(),
        prompt_version: "v1".to_string(),
        prompt_hash: "hash".to_string(),
        symbol: symbol.to_string(),
        timeframe: "15m".to_string(),
        side: "long".to_string(),
        cycle_position: "trend".to_string(),
        detected_patterns: vec!["h2".to_string()],
        entry_price: 100.0,
        stop_price: 99.0,
        target_price: 102.0,
        exit_price: Some(100.0 + r),
        size: 1.0,
        filled: true,
        fill_ratio: 1.0,
        fees_usd: 1.0,
        realized_pnl_usd: pnl_usd,
        pnl_source: "model".to_string(),
        r_multiple: r,
        mfe_r: r.max(0.0),
        mae_r: (-r).max(0.0),
        hold_bars: 5,
        exit_reason: if r > 0.0 { "take_profit".to_string() } else { "stop_loss".to_string() },
        qualified: true,
        qualification_reason: "合格样本".to_string(),
        created_ms: 1000,
        resolved_ms: 2000,
    }
}

#[test]
fn test_multidimensional_metrics_breakdown() {
    let outcomes = vec![
        sample_outcome("2pa_trend", "BTC-USDT-SWAP", 2.0, 200.0),
        sample_outcome("2pa_trend", "ETH-USDT-SWAP", -1.0, -100.0),
        sample_outcome("dog_walking", "BTC-USDT-SWAP", 1.5, 150.0),
        sample_outcome("dog_walking", "ETH-USDT-SWAP", 2.5, 250.0),
    ];

    let by_strat = group_by_strategy(&outcomes);
    assert_eq!(by_strat["2pa_trend"].len(), 2);
    assert_eq!(by_strat["dog_walking"].len(), 2);

    let by_sym = group_by_symbol(&outcomes);
    assert_eq!(by_sym["BTC-USDT-SWAP"].len(), 2);
    assert_eq!(by_sym["ETH-USDT-SWAP"].len(), 2);

    let multi = compute_multidimensional_metrics(&outcomes);
    assert_eq!(multi.overall.samples, 4);
    assert_eq!(multi.by_strategy["2pa_trend"].samples, 2);
    assert_eq!(multi.by_strategy["dog_walking"].samples, 2);
    assert_eq!(multi.by_symbol["BTC-USDT-SWAP"].samples, 2);
    assert_eq!(multi.by_symbol["ETH-USDT-SWAP"].samples, 2);
    assert!(multi.by_strategy["dog_walking"].win_rate > 0.99);
}

#[test]
fn test_daily_drawdown_circuit_breaker_lifecycle() {
    let guard = Arc::new(DailyDrawdownGuardHook::new(200.0));
    let mut pipeline = HookPipeline::new();
    pipeline.add_hook(guard.clone());

    let pre_anal = PreAnalysisContext {
        symbol: "BTC-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa".to_string(),
        timestamp_ms: 1000,
        account_equity_usd: Some(5000.0),
    };

    assert!(pipeline.run_pre_analysis(&pre_anal).is_ok());

    // Record loss reaching 250 USD (> 200 threshold)
    guard.record_loss(250.0);
    assert!(guard.is_tripped());

    // Analysis is now blocked by circuit breaker
    let err = pipeline.run_pre_analysis(&pre_anal).unwrap_err();
    assert!(matches!(err, HookRejection::CircuitBreakerTriggered(_)));

    // Order execution is also blocked
    let pre_exec = PreExecutionContext {
        symbol: "BTC-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa".to_string(),
        decision: serde_json::json!({"action": "OPEN"}),
        account_balance_usd: 5000.0,
        is_shadow_mode: false,
    };
    let exec_err = pipeline.run_pre_execution(&pre_exec).unwrap_err();
    assert!(matches!(exec_err, HookRejection::CircuitBreakerTriggered(_)));
}

#[test]
fn test_shadow_trading_hook_matching() {
    let shadow = Arc::new(ShadowTradingHook::new());
    let mut pipeline = HookPipeline::new();
    pipeline.add_hook(shadow.clone());

    let pre_exec = PreExecutionContext {
        symbol: "ETH-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa".to_string(),
        decision: serde_json::json!({
            "action": "OPEN",
            "order_direction": "做空",
            "entry_price": 3000.0,
            "stop_loss_price": 3050.0,
            "take_profit_price": 2900.0
        }),
        account_balance_usd: 10000.0,
        is_shadow_mode: true,
    };

    let action = pipeline.run_pre_execution(&pre_exec).expect("shadow execution");
    assert!(matches!(action, HookAction::InterceptShadow { .. }));

    let bar = KlineBar {
        seq: 1,
        ts_open: 2000,
        open: 2980.0,
        high: 3010.0,
        low: 2890.0,
        close: 2895.0,
        volume: 500.0,
        amount: 0.0,
        pct_chg: None,
        closed: true,
    };

    let resolved = shadow.match_bar("ETH-USDT-SWAP", &bar);
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].exit_reason, "take_profit");
    assert!(resolved[0].r_multiple >= 2.0);
}

#[test]
fn test_hook_pipeline_end_to_end_drawdown_and_shadow() {
    let guard = Arc::new(DailyDrawdownGuardHook::new(100.0));
    let shadow = Arc::new(ShadowTradingHook::new());
    let mut pipeline = HookPipeline::new();
    pipeline.add_hook(guard.clone());
    pipeline.add_hook(shadow.clone());

    let pre_anal = PreAnalysisContext {
        symbol: "BTC-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa".to_string(),
        timestamp_ms: 1000,
        account_equity_usd: Some(5000.0),
    };
    assert!(pipeline.run_pre_analysis(&pre_anal).is_ok());

    // Submit shadow order
    let pre_exec = PreExecutionContext {
        symbol: "BTC-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa".to_string(),
        decision: serde_json::json!({
            "action": "OPEN",
            "order_direction": "做多",
            "entry_price": 50000.0,
            "stop_loss_price": 49000.0,
            "take_profit_price": 52000.0
        }),
        account_balance_usd: 5000.0,
        is_shadow_mode: true,
    };

    let act = pipeline.run_pre_execution(&pre_exec).expect("shadow intercept");
    assert!(matches!(act, HookAction::InterceptShadow { .. }));

    // Bar hits stop loss: entry 50000, low 48500 <= sl 49000
    let losing_bar = KlineBar {
        seq: 1,
        ts_open: 2000,
        open: 49800.0,
        high: 50100.0,
        low: 48500.0,
        close: 48900.0,
        volume: 10.0,
        amount: 0.0,
        pct_chg: None,
        closed: true,
    };

    let resolved = shadow.match_bar("BTC-USDT-SWAP", &losing_bar);
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].exit_reason, "stop_loss");
    assert!(resolved[0].r_multiple <= -1.0);

    // Feed outcome to pipeline
    pipeline.run_post_outcome(&okx_2pa_agent::learning::PostOutcomeContext {
        outcome: resolved[0].clone(),
    }).expect("post outcome");

    // Guard should record loss: realized_pnl is r_mult * 100 = -100 USD, so loss = 100 USD >= max_loss (100)
    assert!(guard.is_tripped());
    assert!(guard.current_drawdown_usd() >= 100.0);

    // Subsequent pre_analysis must be rejected
    let err = pipeline.run_pre_analysis(&pre_anal).unwrap_err();
    assert!(matches!(err, HookRejection::CircuitBreakerTriggered(_)));

    // Reset clears breaker
    guard.reset();
    assert!(!guard.is_tripped());
    assert!(pipeline.run_pre_analysis(&pre_anal).is_ok());
}

