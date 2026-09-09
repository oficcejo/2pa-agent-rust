mod support;
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
    let mut routes = support::routes();
    routes.insert("/api/v5/public/instruments".into(), support::ok(serde_json::json!([{
        "tickSz":"0.1", "lotSz":"0.00000001", "minSz":"0.00001", "quoteCcy":"USDT"
    }])));
    let mock = support::Mock::start(routes).await;
    let client = mock.client.clone();
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
        true,
        2.0,
        25.0,
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
    let (req, is_algo) = executor.build_request("BTC-USDT", &decision, signal_id).await.expect("valid mock request must build");
    assert_eq!(req["tag"], BROKER_TAG);
    assert_eq!(req["side"], "buy");
    assert!(!is_algo);
    assert!(req["attachAlgoOrds"][0]["attachAlgoClOrdId"].as_str().unwrap().starts_with("pa"));
}

#[tokio::test]
async fn test_build_request_rejects_ultra_narrow_stop_loss() {
    let client = OKXClient::new("http://127.0.0.1:1", None, true, 5);
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
        true,
        2.0,
        25.0,
    );

    // Stop distance is 17 points on 79755 (0.021%)
    let narrow_decision = serde_json::json!({
        "order_direction": "做多",
        "order_type": "限价单",
        "entry_price": 79755.0,
        "stop_loss_price": 79738.0,
        "take_profit_price": 80300.0,
        "trade_confidence": 75,
    });

    let res = executor.build_request("BTC-USDT", &narrow_decision, "sig_narrow_test").await;
    assert!(res.is_err());
    let err_str = res.unwrap_err().to_string();
    assert!(err_str.contains("止损距离过窄"));
}

