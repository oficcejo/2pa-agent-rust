use okx_2pa_agent::okx::client::OKXClient;
use okx_2pa_agent::okx::trading::{OKXTradeExecutor, BROKER_TAG};

#[test]
fn test_broker_tag_constant() {
    assert_eq!(BROKER_TAG, "c314b0aecb5bBCDE");
}

#[test]
fn test_signal_id_generation() {
    let decision = serde_json::json!({
        "order_direction": "做多",
        "order_type": "限价单",
        "entry_price": 60000.0,
        "stop_loss_price": 59000.0,
        "take_profit_price": 62000.0,
    });

    let sig1 = OKXTradeExecutor::generate_signal_id("BTC-USDT", "15m", 1700000000000, &decision);
    let sig2 = OKXTradeExecutor::generate_signal_id("BTC-USDT", "15m", 1700000000000, &decision);
    assert_eq!(sig1, sig2);
    assert_eq!(sig1.len(), 24);
}

#[tokio::test]
async fn test_build_request_contains_broker_tag() {
    let client = OKXClient::new("https://www.okx.com", None, true, 10);
    let executor = OKXTradeExecutor::new(
        client,
        1.0,
        "cross",
        "net",
        3.0,
        true,
        40,
        120,
        3,
        None,
    );

    let decision = serde_json::json!({
        "order_direction": "做多",
        "order_type": "限价单",
        "entry_price": 60000.0,
        "stop_loss_price": 59000.0,
        "take_profit_price": 62000.0,
        "trade_confidence": 75,
    });

    let signal_id = "test_signal_123456";
    // Note: build_request queries OKX instruments public API
    if let Ok((req, is_algo)) = executor.build_request("BTC-USDT", &decision, signal_id).await {
        assert_eq!(req.get("tag").and_then(|v| v.as_str()), Some("c314b0aecb5bBCDE"));
        assert_eq!(req.get("side").and_then(|v| v.as_str()), Some("buy"));
        assert!(!is_algo);
    }
}
