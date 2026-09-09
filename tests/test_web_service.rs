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

