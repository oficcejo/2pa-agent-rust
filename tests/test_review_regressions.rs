//! Regression coverage for the 2026-09-09 review fixes.
//! Run explicitly: cargo test --test test_review_regressions -- --nocapture
//! Uses synthetic data and a loopback GET-only mock; never places exchange orders.

use axum::{routing::get, Json, Router};
use okx_2pa_agent::okx::{
    client::{OKXClient, OKXCredentials},
    trading::OKXTradeExecutor,
};
use rust_decimal_macros::dec;
use serde_json::json;

fn executor(client: OKXClient, leverage: f64, margin: f64) -> OKXTradeExecutor {
    OKXTradeExecutor::new(
        client, 35.0, "cross", "net", leverage, true, 40, 120, 3, None, true, 2.0, margin,
    )
}

async fn mock_balance(available: &str) -> (OKXClient, tokio::task::JoinHandle<()>) {
    let body = json!({"code":"0", "data":[{"totalEq":"100", "details":[{
        "ccy":"USDT", "availEq":available, "availBal":available
    }]}]});
    let app = Router::new().route(
        "/api/v5/account/balance",
        get(move || async move { Json(body) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = OKXClient::new(
        &url,
        Some(OKXCredentials::new("test", "test", "test")),
        true,
        5,
    );
    (client, task)
}



#[tokio::test]
async fn minimum_lot_must_not_override_risk_budget() {
    let (client, task) = mock_balance("100").await;
    let result = executor(client, 10.0, 100.0)
        .compute_order_size(
            "TEST-USDT-SWAP",
            "SWAP",
            dec!(100),
            dec!(50),
            dec!(1),
            dec!(1),
            &json!({"ctVal":"1", "ctType":"linear"}),
        )
        .await;
    task.abort();
    assert!(
        result.is_err(),
        "risk budget is $2 but minimum lot risks $50: {result:?}"
    );
}

#[tokio::test]
async fn minimum_lot_must_not_override_margin_budget() {
    let (client, task) = mock_balance("100").await;
    let result = executor(client, 3.0, 25.0)
        .compute_order_size(
            "TEST-USDT-SWAP",
            "SWAP",
            dec!(100),
            dec!(99),
            dec!(1),
            dec!(1),
            &json!({"ctVal":"1", "ctType":"linear"}),
        )
        .await;
    task.abort();
    assert!(
        result.is_err(),
        "margin budget is $25 but minimum lot needs $33.33: {result:?}"
    );
}

#[tokio::test]
async fn zero_available_margin_must_block_new_size() {
    let (client, task) = mock_balance("0").await;
    let result = executor(client, 3.0, 25.0)
        .compute_order_size(
            "TEST-USDT-SWAP",
            "SWAP",
            dec!(100),
            dec!(99),
            dec!(0.01),
            dec!(0.01),
            &json!({"ctVal":"1", "ctType":"linear"}),
        )
        .await;
    task.abort();
    assert!(
        result.is_err(),
        "no available margin must not use $100 total equity: {result:?}"
    );
}

#[tokio::test]
async fn signal_older_than_120_seconds_must_expire() {
    let client = OKXClient::new("http://127.0.0.1:1", None, true, 5);
    // Missing confidence ensures this never proceeds to any HTTP request.
    let decision = json!({"order_type":"市价单", "order_direction":"做多"});
    let bar_open = chrono::Utc::now().timestamp_millis() - 900_000 - 180_000;
    let result = executor(client, 3.0, 25.0)
        .execute("TEST-USDT-SWAP", "15m", bar_open, &decision)
        .await;
    assert!(
        result.reason.contains("信号已过期"),
        "3-minute-old signal passed 120-second gate: {}",
        result.reason
    );
}


