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

    // Switch to dog_trend
    *service.current_trading_system.write() = "dog_trend".to_string();
    let st_dog_trend = service.status();
    assert_eq!(st_dog_trend.get("trading_system").and_then(|v| v.as_str()), Some("dog_trend"));

    // Verify available_trading_systems includes dog_trend and does not contain alpha_pilot
    let avail = st_dog_trend.get("available_trading_systems").and_then(|v| v.as_array()).unwrap();
    let has_dog_trend = avail.iter().any(|s| s.get("id").and_then(|v| v.as_str()) == Some("dog_trend"));
    assert!(has_dog_trend, "available_trading_systems must contain dog_trend");
    let has_alpha = avail.iter().any(|s| s.get("id").and_then(|v| v.as_str()) == Some("alpha_pilot"));
    assert!(!has_alpha, "available_trading_systems must not contain alpha_pilot");

    // Switch back to 2pa
    *service.current_trading_system.write() = "2pa".to_string();
    let st3 = service.status();
    assert_eq!(st3.get("trading_system").and_then(|v| v.as_str()), Some("2pa"));
}

#[test]
fn test_alpha_pilot_config_fallback_to_2pa_trend() {
    let mut settings = Settings::default();
    settings.general.trading_system = "alpha_pilot".to_string();

    let service = WebTradingService::new(settings);
    let st = service.status();
    assert_eq!(st.get("trading_system").and_then(|v| v.as_str()), Some("2pa_trend"),
        "deprecated alpha_pilot in settings should automatically fallback to 2pa_trend");
}

#[test]
fn test_historical_alpha_pilot_record_deserialization() {
    let raw_json = serde_json::json!({
        "meta": {
            "timestamp_local_iso": "2026-09-09T12:00:00+08:00",
            "timestamp_local_ms": 1700000000000i64,
            "symbol": "ETH-USDT-SWAP",
            "timeframe": "15m",
            "bar_count": 800,
            "trading_system": "alpha_pilot"
        },
        "stage2_decision": {
            "trading_system": "alpha_pilot",
            "decision": {
                "action": "WAIT",
                "order_type": "不下单",
                "trade_confidence": 0
            }
        }
    });

    let record: okx_2pa_agent::records::schema::AnalysisRecord = serde_json::from_value(raw_json)
        .expect("historical alpha_pilot record must deserialize without errors");
    assert_eq!(record.meta.trading_system, "alpha_pilot");
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

