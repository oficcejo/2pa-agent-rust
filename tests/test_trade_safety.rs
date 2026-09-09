mod support;
use okx_2pa_agent::{
    config::settings::Settings,
    data::base::PositionContext,
    okx::trading::OKXTradeExecutor,
    web::{positions::*, server::create_router, service::WebTradingService},
};
use serde_json::{json, Value};
use std::sync::Arc;
use support::{ok, routes, Mock};
use tower::ServiceExt;

fn executor(mock: &Mock) -> OKXTradeExecutor {
    OKXTradeExecutor::new(
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
    )
}
fn decision() -> Value {
    json!({"action":"OPEN", "order_type":"限价单", "order_direction":"做多", "entry_price":100.0,
    "stop_loss_price":98.0, "take_profit_price":104.0,"trade_confidence":75})
}

#[tokio::test]
async fn account_error_blocks_order_and_leverage_changes() {
    let mut responses = routes();
    responses.insert(
        "/api/v5/account/positions".into(),
        json!({"code":"50011","msg":"unavailable","data":[]}),
    );
    let mock = Mock::start(responses).await;
    let res = executor(&mock)
        .execute(
            "TEST-USDT-SWAP",
            "15m",
            chrono::Utc::now().timestamp_millis() - 900_000,
            &decision(),
        )
        .await;
    assert!(!res.submitted);
    assert!(res.reason.contains("50011"));
    assert!(mock
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|(method, _, _)| method == "GET"));
}

#[tokio::test]
async fn leverage_error_blocks_submission_even_when_no_position() {
    let mut responses = routes();
    responses.insert(
        "/api/v5/account/set-leverage".into(),
        json!({"code":"51000","msg":"leverage rejected","data":[]}),
    );
    let mock = Mock::start(responses).await;
    let res = executor(&mock)
        .execute(
            "TEST-USDT-SWAP",
            "15m",
            chrono::Utc::now().timestamp_millis() - 900_000,
            &decision(),
        )
        .await;
    assert!(!res.submitted);
    assert!(res.reason.contains("51000"));
    assert!(!mock
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|(m, p, _)| m == "POST" && p.starts_with("/api/v5/trade/order?")));
}

#[tokio::test]
async fn simultaneous_identical_signal_submits_only_once_and_keeps_manual_orders() {
    let mut responses = routes();
    responses.insert(
        "/api/v5/trade/orders-pending".into(),
        ok(json!([{"ordId":"manual","clOrdId":"manual123"}])),
    );
    let mock = Mock::start(responses).await;
    let ex = executor(&mock);
    let timestamp = chrono::Utc::now().timestamp_millis() - 900_000;
    let d = decision();
    let (a, b) = tokio::join!(
        ex.execute("TEST-USDT-SWAP", "15m", timestamp, &d),
        ex.execute("TEST-USDT-SWAP", "15m", timestamp, &d)
    );
    assert_eq!(
        a.submitted as usize + b.submitted as usize,
        1,
        "{a:?} {b:?}"
    );
    let calls = mock.calls.lock().unwrap();
    assert!(!calls.iter().any(|(_, p, _)| p.contains("cancel")));
    assert_eq!(
        calls
            .iter()
            .filter(|(m, p, _)| m == "POST" && p.starts_with("/api/v5/trade/order?"))
            .count(),
        1
    );
}

fn held_short() -> Value {
    json!({"instId":"TEST-USDT-SWAP","pos":"2","posSide":"short","avgPx":"100","markPx":"98","mgnMode":"cross","upl":"4","uplRatio":"0.02"})
}
fn protection() -> Value {
    json!({"instId":"TEST-USDT-SWAP","algoId":"protect","algoClOrdId":"paprotect","side":"buy","posSide":"short","slTriggerPx":"103","tpTriggerPx":"90"})
}

#[tokio::test]
async fn amend_failure_preserves_old_protection_and_rejects_wider_combined_stop() {
    let mut responses = routes();
    responses.insert(
        "/api/v5/account/positions".into(),
        ok(json!([held_short()])),
    );
    responses.insert(
        "/api/v5/trade/orders-algo-pending?ordType=conditional&instId=TEST-USDT-SWAP".into(),
        ok(json!([protection()])),
    );
    responses.insert(
        "/api/v5/trade/amend-algos".into(),
        ok(json!([{"sCode":"51000","sMsg":"amend failed"}])),
    );
    let mock = Mock::start(responses).await;
    let wider =
        json!({"action":"MOVE_SL_TP","new_stop_loss_price":104.0,"new_take_profit_price":90.0});
    let err = execute_management(&mock.client, "TEST-USDT-SWAP", "long_short", &wider)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("只能收紧"));
    assert!(mock
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|(m, _, _)| m == "GET"));
    let tighter = json!({"action":"MOVE_STOP_LOSS","new_stop_loss_price":101.0});
    assert!(
        execute_management(&mock.client, "TEST-USDT-SWAP", "long_short", &tighter)
            .await
            .is_err()
    );
    let calls = mock.calls.lock().unwrap();
    let posts: Vec<_> = calls.iter().filter(|(m, _, _)| m == "POST").collect();
    assert_eq!(posts.len(), 1);
    assert!(posts[0].1.starts_with("/api/v5/trade/amend-algos?"));
    assert_eq!(posts[0].2["algoId"], "protect");
    assert!(posts[0].2.get("newTpTriggerPx").is_none());
}

#[tokio::test]
async fn close_hedged_short_passes_short_position_side() {
    let mut responses = routes();
    responses.insert(
        "/api/v5/account/positions".into(),
        ok(json!([held_short()])),
    );
    responses.insert(
        "/api/v5/trade/close-position".into(),
        ok(json!([{"instId":"TEST-USDT-SWAP","posSide":"short"}])),
    );
    let mock = Mock::start(responses).await;
    execute_management(
        &mock.client,
        "TEST-USDT-SWAP",
        "long_short",
        &json!({"action":"CLOSE_EARLY"}),
    )
    .await
    .unwrap();
    let calls = mock.calls.lock().unwrap();
    let post = calls
        .iter()
        .find(|(m, p, _)| m == "POST" && p.starts_with("/api/v5/trade/close-position?"))
        .unwrap();
    assert_eq!(post.2["posSide"], "short");
}

#[test]
fn normalized_position_and_trailing_stop_handle_both_modes() {
    let row = normalize_position(&held_short()).unwrap();
    assert_eq!(row["direction"], "short");
    assert_eq!(row["unrealized_pnl"], 4.0);
    assert_eq!(row["instrument"], "TEST-USDT-SWAP");
    let pos = PositionContext {
        pos_side: "long".into(),
        current_sl: Some(95.0),
        mark_px: Some(100.0),
        ..Default::default()
    };
    assert!(validate_stop_change(&pos, 97.0).is_ok());
    for sl in [94.0, 100.0, f64::NAN] {
        assert!(validate_stop_change(&pos, sl).is_err());
    }
    assert_eq!(
        position_side(&json!({"pos":"-2","posSide":"net"})).unwrap(),
        "short"
    );
}

#[tokio::test]
async fn private_routes_require_auth_and_reject_cross_site_writes() {
    let settings = Settings {
        web_auth_token: "test-password".into(),
        ..Default::default()
    };
    let app = create_router(Arc::new(WebTradingService::new(settings)));
    for path in [
        "/",
        "/api/status",
        "/api/account",
        "/api/history/trades",
        "/api/config",
    ] {
        let req = axum::http::Request::builder()
            .uri(path)
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(app.clone().oneshot(req).await.unwrap().status(), 401);
    }
    let req = axum::http::Request::builder()
        .uri("/api/status")
        .header("Authorization", "Bearer test-password")
        .body(axum::body::Body::empty())
        .unwrap();
    assert_eq!(app.clone().oneshot(req).await.unwrap().status(), 200);
    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/api/automation")
        .header("Authorization", "Bearer test-password")
        .header("Content-Type", "application/json")
        .header("Sec-Fetch-Site", "cross-site")
        .body(axum::body::Body::from("{}"))
        .unwrap();
    assert_eq!(app.oneshot(req).await.unwrap().status(), 403);
}

#[tokio::test]
async fn execution_disabled_is_rejected_before_any_network_request() {
    let service = WebTradingService::new(Settings::default());
    let err = service
        .analyze("TEST-USDT-SWAP", "15m", 100, true, Some("alpha_pilot"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("凭据未配置"));
    let mut settings = Settings::default();
    settings.okx.api_key = "test".into();
    settings.okx.secret_key = "test".into();
    settings.okx.passphrase = "test".into();
    settings.okx.demo_trading = false;
    settings.okx.auto_trading_enabled = true;
    assert!(WebTradingService::new(settings)
        .ensure_execution_enabled()
        .unwrap_err()
        .to_string()
        .contains("未授权"));
}

#[tokio::test]
async fn expiry_only_cancels_old_owned_entry_orders() {
    let mut responses = routes();
    let old = (chrono::Utc::now().timestamp_millis() - 3_000_000).to_string();
    let fresh = chrono::Utc::now().timestamp_millis().to_string();
    responses.insert(
        "/api/v5/trade/orders-pending".into(),
        ok(json!([
            {"ordId":"old-entry","clOrdId":"paold","cTime":old},
            {"ordId":"manual","clOrdId":"manual","cTime":old},
            {"ordId":"fresh","clOrdId":"pafresh","cTime":fresh}
        ])),
    );
    responses.insert(
        "/api/v5/trade/orders-algo-pending".into(),
        ok(json!([
            {"algoId":"old-trigger","algoClOrdId":"paold","cTime":old}
        ])),
    );
    responses.insert(
        "/api/v5/trade/cancel-order".into(),
        ok(json!([{"sCode":"0"}])),
    );
    responses.insert(
        "/api/v5/trade/cancel-algos".into(),
        ok(json!([{"sCode":"0"}])),
    );
    let mock = Mock::start(responses).await;
    assert_eq!(
        executor(&mock)
            .cancel_expired_entries("TEST-USDT-SWAP", "15m")
            .await
            .unwrap(),
        2
    );
    let calls = mock.calls.lock().unwrap();
    let posts: Vec<_> = calls.iter().filter(|(m, _, _)| m == "POST").collect();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0].2["ordId"], "old-entry");
    assert_eq!(posts[1].2[0]["algoId"], "old-trigger");
    assert!(!calls
        .iter()
        .any(|(_, p, _)| p.contains("conditional") || p.contains("oco")));
}

#[tokio::test]
async fn account_payload_uses_ui_fields_and_actual_available_margin() {
    let mut responses = routes();
    responses.insert(
        "/api/v5/account/balance".into(),
        ok(json!([{"totalEq":"1000","details":[{
            "ccy":"USDT","eq":"1000","availEq":"0","availBal":"0","frozenBal":"1000"
        }]}])),
    );
    responses.insert(
        "/api/v5/account/positions".into(),
        ok(json!([held_short()])),
    );
    let mock = Mock::start(responses).await;
    let mut settings = Settings::default();
    settings.okx.api_key = "test".into();
    settings.okx.secret_key = "test".into();
    settings.okx.passphrase = "test".into();
    let service = WebTradingService::new(settings);
    *service.okx_client.write() = mock.client.clone();
    let account = service.account().await.unwrap();
    assert_eq!(account["summary"]["total_equity_usd"], 1000.0);
    assert_eq!(account["summary"]["available_equity_usd"], 0.0);
    assert_eq!(account["positions"][0]["direction"], "short");
    assert_eq!(account["balances"][0]["currency"], "USDT");
    assert_eq!(account["balances"][0]["frozen"], 1000.0);
    assert_eq!(account["equity_curve"][0]["value"], 1000.0);
}

#[tokio::test]
async fn valid_size_respects_risk_and_margin_after_estimated_fees() {
    use rust_decimal_macros::dec;
    let mock = Mock::start(routes()).await;
    let size = executor(&mock)
        .compute_order_size(
            "TEST-USDT-SWAP",
            "SWAP",
            dec!(100),
            dec!(98),
            dec!(0.01),
            dec!(0.01),
            &json!({"ctVal":"1","ctType":"linear"}),
        )
        .await
        .unwrap();
    assert!(size > dec!(0));
    assert!(size * dec!(2.1) <= dec!(20));
    assert!(size * (dec!(100) / dec!(3) + dec!(0.1)) <= dec!(250));
    assert_eq!(size % dec!(0.01), dec!(0));
}

#[tokio::test]
async fn moved_market_price_invalidates_old_stop_before_submission() {
    let mut responses = routes();
    responses.insert("/api/v5/market/ticker".into(), ok(json!([{"last":"97"}])));
    let mock = Mock::start(responses).await;
    let mut d = decision();
    d["order_type"] = json!("市价单");
    let result = executor(&mock)
        .build_request("TEST-USDT-SWAP", &d, "test")
        .await;
    assert!(result.unwrap_err().to_string().contains("价格关系异常"));
    assert!(mock
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|(m, _, _)| m == "GET"));
}

#[test]
fn rolling_normalization_is_causal_across_warmup_boundary() {
    use okx_2pa_agent::indicators::alpha_pilot::rolling_zscore_500;
    let prefix: Vec<_> = (0..499).map(|i| (i as f64).sin()).collect();
    let before = rolling_zscore_500(&prefix);
    let mut extended = prefix.clone();
    extended.extend([1000.0, -1000.0]);
    let after = rolling_zscore_500(&extended);
    assert_eq!(before, &after[..499]);
}
