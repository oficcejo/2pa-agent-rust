use okx_2pa_agent::learning::hooks::{
    HookAction, HookPipeline, HookRejection, PreExecutionContext, TradingHook,
    TypeSafeConfidenceGuardHook,
};
use serde_json::json;
use std::sync::Arc;

#[test]
fn test_typesafe_confidence_guard_lifecycle() {
    let guard = Arc::new(TypeSafeConfidenceGuardHook::new(0.70));
    assert_eq!(guard.name(), "TypeSafeConfidenceGuardHook");

    let mut pipeline = HookPipeline::new();
    pipeline.add_hook(guard.clone());

    // Case 1: Non-OPEN actions always pass through
    let ctx_wait = PreExecutionContext {
        symbol: "BTC-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa_trend".to_string(),
        decision: json!({
            "action": "WAIT",
            "typesafe_confidence": 0.20
        }),
        account_balance_usd: 5000.0,
        is_shadow_mode: false,
    };
    assert_eq!(
        pipeline.run_pre_execution(&ctx_wait).unwrap(),
        HookAction::Proceed
    );

    // Case 2: OPEN action without TypeSafe metadata passes through
    let ctx_legacy = PreExecutionContext {
        symbol: "BTC-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa_trend".to_string(),
        decision: json!({
            "action": "OPEN",
            "entry_price": 60000.0
        }),
        account_balance_usd: 5000.0,
        is_shadow_mode: false,
    };
    assert_eq!(
        pipeline.run_pre_execution(&ctx_legacy).unwrap(),
        HookAction::Proceed
    );

    // Case 3: OPEN action with high TypeSafe confidence passes
    let ctx_high_conf = PreExecutionContext {
        symbol: "BTC-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa_trend".to_string(),
        decision: json!({
            "action": "OPEN",
            "typesafe_confidence": 0.82,
            "entry_price": 60000.0
        }),
        account_balance_usd: 5000.0,
        is_shadow_mode: false,
    };
    assert_eq!(
        pipeline.run_pre_execution(&ctx_high_conf).unwrap(),
        HookAction::Proceed
    );

    // Case 4: OPEN action with low TypeSafe confidence gets rejected
    let ctx_low_conf = PreExecutionContext {
        symbol: "BTC-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa_trend".to_string(),
        decision: json!({
            "action": "OPEN",
            "typesafe_confidence": 0.58,
            "entry_price": 60000.0
        }),
        account_balance_usd: 5000.0,
        is_shadow_mode: false,
    };
    let res = pipeline.run_pre_execution(&ctx_low_conf);
    assert!(res.is_err());
    match res.unwrap_err() {
        HookRejection::RiskLimitExceeded(msg) => {
            assert!(msg.contains("0.58"));
            assert!(msg.contains("0.70"));
        }
        other => panic!("Expected RiskLimitExceeded, got {:?}", other),
    }

    // Case 5: Confidence nested in stage1_diagnosis is correctly detected and rejected
    let ctx_nested_low_conf = PreExecutionContext {
        symbol: "BTC-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa_trend".to_string(),
        decision: json!({
            "action": "OPEN",
            "stage1_diagnosis": {
                "typesafe_confidence": 0.61
            }
        }),
        account_balance_usd: 5000.0,
        is_shadow_mode: false,
    };
    assert!(pipeline.run_pre_execution(&ctx_nested_low_conf).is_err());

    // Case 6: Dynamic threshold adjustment works in real time
    guard.set_min_confidence(0.50);
    assert_eq!(
        pipeline.run_pre_execution(&ctx_low_conf).unwrap(),
        HookAction::Proceed
    );
}
