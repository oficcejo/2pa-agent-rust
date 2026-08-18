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

    // Switch back to 2pa
    *service.current_trading_system.write() = "2pa".to_string();
    let st3 = service.status();
    assert_eq!(st3.get("trading_system").and_then(|v| v.as_str()), Some("2pa"));
}
