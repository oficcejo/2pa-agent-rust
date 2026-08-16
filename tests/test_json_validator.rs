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
}
