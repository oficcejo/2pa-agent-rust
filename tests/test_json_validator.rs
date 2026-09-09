use okx_2pa_agent::ai::json_validator::{
    extract_outer_json_object, strip_markdown_fences, validate_stage1_json,
    validate_stage2_json,
};

#[test]
fn test_strip_markdown_fences() {
    let fenced = "```json\n{\"test\": 123}\n```";
    assert_eq!(strip_markdown_fences(fenced), "{\"test\": 123}");
}

#[test]
fn test_extract_outer_json() {
    let mixed = "思考：\n接下来输出 JSON\n{\"cycle_position\": \"spike\", \"gate_result\": \"proceed\"}\n请查看。";
    let extracted = extract_outer_json_object(mixed);
    assert_eq!(
        extracted,
        "{\"cycle_position\": \"spike\", \"gate_result\": \"proceed\"}"
    );
}

#[test]
fn test_stage1_validation() {
    let valid_stage1 = serde_json::json!({
        "cycle_position": "spike",
        "dominant_force": "bulls",
        "gate_result": "proceed",
        "trend_state": "strong_bull",
    });
    assert!(validate_stage1_json(&valid_stage1, "").is_ok());

    let invalid_stage1 = serde_json::json!({
        "cycle_position": "spike",
    });
    assert!(validate_stage1_json(&invalid_stage1, "").is_err());
}

#[test]
fn test_stage2_validation() {
    let valid_stage2 = serde_json::json!({
        "decision": {
            "order_type": "限价单",
            "order_direction": "做多",
            "entry_price": 50000.0,
            "stop_loss_price": 49000.0,
            "take_profit_price": 52000.0,
            "trade_confidence": 80
        }
    });
    assert!(validate_stage2_json(&valid_stage2, "").is_ok());

    // Bad stop loss for buy
    let invalid_stop_loss = serde_json::json!({
        "decision": {
            "order_type": "限价单",
            "order_direction": "做多",
            "entry_price": 50000.0,
            "stop_loss_price": 51000.0, // Error: stop loss > entry
            "take_profit_price": 52000.0,
            "trade_confidence": 80
        }
    });
    assert!(validate_stage2_json(&invalid_stop_loss, "").is_err());

    // Valid MOVE_STOP_LOSS
    let valid_move_sl = serde_json::json!({
        "decision": {
            "action": "MOVE_STOP_LOSS",
            "order_type": "修改止损",
            "order_direction": "做空",
            "new_stop_loss_price": 2280.0,
            "trade_confidence": 90
        }
    });
    assert!(validate_stage2_json(&valid_move_sl, "").is_ok());

    // Valid CLOSE_EARLY
    let valid_close_early = serde_json::json!({
        "decision": {
            "action": "CLOSE_EARLY",
            "order_type": "平仓",
            "order_direction": "做空",
            "trade_confidence": 85
        }
    });
    assert!(validate_stage2_json(&valid_close_early, "").is_ok());

    // Valid HOLD
    let valid_hold = serde_json::json!({
        "decision": {
            "action": "HOLD",
            "order_type": "持有",
            "order_direction": "做多",
            "trade_confidence": 80
        }
    });
    assert!(validate_stage2_json(&valid_hold, "").is_ok());
}

#[test]
fn test_ultra_narrow_stop_loss_rejected() {
    // Ultra-narrow stop loss: entry 80000, stop 79980 (distance 20, 0.025%)
    let narrow_sl = serde_json::json!({
        "decision": {
            "order_type": "限价单",
            "order_direction": "做多",
            "entry_price": 80000.0,
            "stop_loss_price": 79980.0,
            "take_profit_price": 80500.0,
            "trade_confidence": 70
        }
    });
    let res = validate_stage2_json(&narrow_sl, "");
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.invalid_fields.iter().any(|f| f.contains("止损距离过窄")));
}

#[test]
fn test_stage1_semantic_conflict_double_top_rejected() {
    let st1 = serde_json::json!({
        "cycle_position": "trading_range",
        "dominant_force": "neutral",
        "gate_result": "proceed",
        "detected_patterns": ["double_top_candidate", "rejection_at_high"]
    });

    let st2_buy = serde_json::json!({
        "decision": {
            "order_type": "限价单",
            "order_direction": "做多",
            "entry_price": 80000.0,
            "stop_loss_price": 79500.0,
            "take_profit_price": 81000.0,
            "trade_confidence": 70
        }
    });

    use okx_2pa_agent::ai::json_validator::validate_stage2_json_with_stage1;
    let res = validate_stage2_json_with_stage1(&st2_buy, "", Some(&st1));
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.invalid_fields.iter().any(|f| f.contains("双顶") || f.contains("高位受阻")));
}

#[test]
fn test_stage1_strong_bear_knife_catching_rejected() {
    let st1 = serde_json::json!({
        "cycle_position": "overstretched_bearish",
        "dominant_force": "bears",
        "gate_result": "proceed",
        "detected_patterns": ["bearish_spike", "strong_trend"]
    });

    let st2_buy = serde_json::json!({
        "decision": {
            "order_type": "限价单",
            "order_direction": "做多",
            "entry_price": 79000.0,
            "stop_loss_price": 78500.0,
            "take_profit_price": 80000.0,
            "trade_confidence": 70
        }
    });

    use okx_2pa_agent::ai::json_validator::validate_stage2_json_with_stage1;
    let res = validate_stage2_json_with_stage1(&st2_buy, "", Some(&st1));
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.invalid_fields.iter().any(|f| f.contains("强空头单边下跌") || f.contains("盲目猜底")));
}

