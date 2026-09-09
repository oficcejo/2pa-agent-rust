//! Regression coverage for the 2026-09-09 review fixes.
//! Run explicitly: cargo test --test test_review_regressions -- --nocapture
//! Uses synthetic data and a loopback GET-only mock; never places exchange orders.

use axum::{routing::get, Json, Router};
use okx_2pa_agent::{
    config::settings::Settings,
    data::{base::KlineBar, snapshot::build_analysis_frame},
    indicators::alpha_pilot::op_jump,
    okx::{
        client::{OKXClient, OKXCredentials},
        trading::OKXTradeExecutor,
    },
    orchestrator::two_stage::TwoStageOrchestrator,
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

#[test]
fn alpha_decision_exposes_executor_confidence() {
    let bars: Vec<_> = (0..800)
        .rev()
        .map(|i| {
            let price = if i % 80 < 40 { 100.0 } else { 90.0 };
            KlineBar {
                seq: 800 - i,
                ts_open: 1_700_000_000_000 + i as i64 * 900_000,
                open: price,
                high: price + 2.0,
                low: price - 2.0,
                close: price + 0.1,
                volume: 100.0,
                amount: 0.0,
                pct_chg: None,
                closed: true,
            }
        })
        .collect();
    let frame = build_analysis_frame(&bars, 800, "ETH-USDT-SWAP", "15m", None).unwrap();
    let orch = TwoStageOrchestrator::new(Settings::default(), std::env::temp_dir());
    let record = orch.run_alpha_pilot_analysis(&frame, None).unwrap();
    let wrapper = record.stage2_decision.unwrap();
    let decision = &wrapper["decision"];
    assert_eq!(
        decision["action"], "OPEN",
        "fixture must exercise an entry signal"
    );
    assert!(
        decision["trade_confidence"].as_u64().is_some(),
        "executor cannot read: {decision}"
    );
    assert!(decision["estimated_win_rate"].is_null());
    use okx_2pa_agent::data::base::PositionContext;
    let same_side = if decision["order_direction"] == "做多" {
        "long"
    } else {
        "short"
    };
    let mut position = PositionContext {
        has_position: true,
        pos_side: same_side.into(),
        ..Default::default()
    };
    let held = orch
        .run_alpha_pilot_analysis(&frame, Some(&position))
        .unwrap();
    assert_eq!(held.stage2_decision.unwrap()["decision"]["action"], "HOLD");
    position.pos_side = if same_side == "long" { "short" } else { "long" }.into();
    let reversed = orch
        .run_alpha_pilot_analysis(&frame, Some(&position))
        .unwrap();
    assert_eq!(
        reversed.stage2_decision.unwrap()["decision"]["action"],
        "CLOSE_EARLY"
    );
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

#[test]
fn jump_history_must_not_change_when_future_bars_are_appended() {
    let prefix = [0.0, 0.0, 0.0, 1.0];
    let old = op_jump(&prefix);
    let mut extended = prefix.to_vec();
    extended.push(100.0);
    let new = op_jump(&extended);
    assert!(
        (old[3] - new[3]).abs() < 1e-12,
        "historical jump changed after adding a future value: {} -> {}",
        old[3],
        new[3]
    );
}
