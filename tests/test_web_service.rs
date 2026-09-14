use okx_2pa_agent::config::settings::Settings;
use okx_2pa_agent::web::service::WebTradingService;

#[test]
fn test_trading_system_switch() {
    let mut settings = Settings::default();
    settings.general.trading_system = "2pa".to_string();

    let service = WebTradingService::new(settings);
    let st1 = service.status();
    assert_eq!(st1.get("trading_system").and_then(|v| v.as_str()), Some("2pa"));

    // Switch to dog_walking
    *service.current_trading_system.write() = "dog_walking".to_string();
    let st2 = service.status();
    assert_eq!(st2.get("trading_system").and_then(|v| v.as_str()), Some("dog_walking"));

    // Switch to alpha_pilot
    *service.current_trading_system.write() = "alpha_pilot".to_string();
    let st_alpha = service.status();
    assert_eq!(st_alpha.get("trading_system").and_then(|v| v.as_str()), Some("alpha_pilot"));

    // Verify available_trading_systems includes alpha_pilot
    let avail = st_alpha.get("available_trading_systems").and_then(|v| v.as_array()).unwrap();
    let has_alpha = avail.iter().any(|s| s.get("id").and_then(|v| v.as_str()) == Some("alpha_pilot"));
    assert!(has_alpha, "available_trading_systems must contain alpha_pilot");

    // Switch back to 2pa
    *service.current_trading_system.write() = "2pa".to_string();
    let st3 = service.status();
    assert_eq!(st3.get("trading_system").and_then(|v| v.as_str()), Some("2pa"));
}

#[tokio::test]
async fn test_alpha_pilot_orchestrator_zero_token_execution() {
    use okx_2pa_agent::data::base::{KlineBar, KlineFrame};
    use okx_2pa_agent::orchestrator::two_stage::TwoStageOrchestrator;


    let settings = Settings::default();
    let test_dir = std::env::temp_dir().join(format!("2pa-test-{}", uuid::Uuid::new_v4()));
    let orch = TwoStageOrchestrator::new(settings, test_dir.clone());

    // Build synthetic K-lines (e.g. 50 bars)
    let mut bars = Vec::new();
    let mut px = 2000.0;
    for i in 0..50 {
        px += 2.0;
        bars.push(KlineBar {
            seq: i + 1,
            ts_open: 1700000000000 + (i as i64) * 900000,
            open: px - 1.0,
            high: px + 5.0,
            low: px - 5.0,
            close: px,
            volume: 100.0,
            amount: 0.0,
            pct_chg: None,
            closed: true,
        });
    }

    let frame = KlineFrame {
        symbol: "ETH-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        bars,
        indicators: okx_2pa_agent::data::base::IndicatorBundle {
            ema20: vec![],
            atr14: vec![],
            sma14: vec![],
            sma170: vec![],
            sma170_slope: vec![],
            dev170_pct: vec![],
        },
        snapshot_ts_local_ms: 1700000000000,
    };

    let record = orch.run_analysis_with_system(&frame, "alpha_pilot").await.expect("AlphaPilot analysis should succeed");
    assert_eq!(record.meta.trading_system, "alpha_pilot");

    // Verify 0 tokens were consumed!
    let usage = record.usage_total;
    assert_eq!(usage.get("total_tokens").and_then(|v| v.as_u64()), Some(0));

    // Verify stage2_decision exists
    let dec = record.stage2_decision.expect("stage2_decision should be present");
    assert_eq!(dec.get("trading_system").and_then(|v| v.as_str()), Some("alpha_pilot"));
    let decision = dec.get("decision").expect("decision field should be present");
    assert!(decision.get("action").is_some());
    assert!(decision.get("trade_confidence").is_some());
    for file in std::fs::read_dir(&test_dir).unwrap() { std::fs::remove_file(file.unwrap().path()).unwrap(); }
    std::fs::remove_dir(&test_dir).unwrap();
}

mod support;

fn candle_row(ts: i64, closed: bool) -> serde_json::Value {
    serde_json::json!([
        ts.to_string(), "100", "101", "99", "100.5", "10", "0", "0",
        if closed { "1" } else { "0" }
    ])
}

fn closed_bar_ts() -> i64 {
    let now = chrono::Utc::now().timestamp_millis();
    now - (now % 900_000) - 900_000
}

async fn automation_service(okx_url: &str, llm_url: Option<&str>) -> (okx_2pa_agent::web::service::WebTradingService, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("2pa-auto-{}", uuid::Uuid::new_v4()));
    let records = dir.join("records");
    std::fs::create_dir_all(&records).unwrap();
    let mut settings = Settings::default();
    settings.okx.base_url = okx_url.to_string();
    settings.okx.api_key = "test".into();
    settings.okx.secret_key = "test".into();
    settings.okx.passphrase = "test".into();
    settings.okx.demo_trading = true;
    settings.okx.auto_trading_enabled = true;
    settings.general.trading_system = "2pa_trend".into();
    settings.general.analysis_bar_count = 20;
    if let Some(url) = llm_url {
        settings.provider.base_url = url.to_string();
        settings.provider.api_key = "test".into();
        settings.validation.retry_max = 0;
    }
    let service = WebTradingService::new(settings);
    *service.automation_enabled.write() = true;
    *service.automation_symbol.write() = "TEST-USDT-SWAP".into();
    *service.automation_timeframe.write() = "15m".into();
    *service.orchestrator.write() = okx_2pa_agent::orchestrator::two_stage::TwoStageOrchestrator::new(
        service.settings.read().clone(),
        records,
    );
    (service, dir)
}

#[tokio::test]
async fn automation_llm_failure_is_recorded_and_does_not_consume_the_bar() {
    let mut routes = support::routes();
    let ts = closed_bar_ts();
    let mut candle_rows = vec![candle_row(ts + 900_000, false)];
    candle_rows.extend((0..25).map(|i| candle_row(ts - i * 900_000, true)));
    routes.insert("/api/v5/market/candles".into(), support::ok(serde_json::Value::Array(candle_rows)));
    let mock = support::Mock::start(routes).await;
    let llm = support::Mock::start(std::collections::HashMap::from([(
        "/v1/chat/completions".into(),
        serde_json::json!({"_http_status":403, "error":{"code":"forbidden","message":"model MiniMax-M3.0 is not allowed in your plan"}}),
    )])).await;
    let (service, _dir) = automation_service(&mock.url, Some(&llm.url)).await;
    let first = service.automation_tick().await;
    assert!(first.is_err(), "{first:?}");
    let status = service.status();
    assert_eq!(status["automation_runtime"]["phase"], "error");
    let err = status["automation_runtime"]["last_error"].as_str().unwrap_or("");
    assert!(err.contains("forbidden") || err.contains("LLM"), "{err}");
    let saved: Vec<_> = std::fs::read_dir(_dir.join("records")).unwrap().filter_map(|e| e.ok()).collect();
    assert_eq!(saved.len(), 1);
    assert!(service.last_closed_ts.read().is_empty());
    let calls_before = llm.calls.lock().unwrap().len();
    service.automation_tick().await.unwrap();
    assert_eq!(llm.calls.lock().unwrap().len(), calls_before, "backoff must suppress repeated LLM calls");
    service.automation_runtime.write().next_retry_ms = None;
    let second = service.automation_tick().await;
    assert!(second.is_err());
    let saved: Vec<_> = std::fs::read_dir(_dir.join("records")).unwrap().filter_map(|e| e.ok()).collect();
    assert_eq!(saved.len(), 2);

    // Recover on the same bar with a valid WAIT decision. A completed bar must
    // not call the model again or create an exchange order on the next tick.
    let reply = serde_json::json!({
        "cycle_position":"trading_range", "dominant_force":"balanced", "gate_result":"wait",
        "decision":{"action":"WAIT", "order_type":"不下单", "order_direction":"不下单", "trade_confidence":0}
    });
    let healthy_llm = support::Mock::start(std::collections::HashMap::from([(
        "/v1/chat/completions".into(),
        serde_json::json!({"choices":[{"message":{"content":reply.to_string()}}]})
    )])).await;
    let mut settings = service.settings.read().clone();
    settings.provider.base_url = healthy_llm.url.clone();
    *service.orchestrator.write() = okx_2pa_agent::orchestrator::two_stage::TwoStageOrchestrator::new(
        settings, _dir.join("records")
    );
    service.automation_runtime.write().next_retry_ms = None;
    service.automation_tick().await.unwrap();
    assert_eq!(service.last_closed_ts.read().get(&("TEST-USDT-SWAP".into(), "15m".into())), Some(&ts));
    assert!(service.status()["automation_runtime"]["last_error"].is_null());
    assert!(service.status()["automation_runtime"]["last_success_ms"].is_number());
    assert_eq!(healthy_llm.calls.lock().unwrap().len(), 2);
    service.automation_tick().await.unwrap();
    assert_eq!(healthy_llm.calls.lock().unwrap().len(), 2);
    assert!(mock.calls.lock().unwrap().iter().all(|(method, _, _)| method == "GET"));
    assert!(mock.calls.lock().unwrap().iter().any(|(_, path, _)| path.contains("bar=1H")),
        "OKX rejects lowercase hourly candle parameters");
    let _ = std::fs::remove_dir_all(_dir);
}

