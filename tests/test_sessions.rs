use chrono::Utc;
use okx_2pa_agent::web::sessions::build_trading_session;

#[test]
fn test_session_always() {
    let session = build_trading_session("always", "UTC", "00:00", "00:00", None);
    assert!(session.is_open_at(Some(Utc::now())));
}

#[test]
fn test_session_presets() {
    let us_reg = build_trading_session("us_regular", "America/New_York", "09:30", "16:00", None);
    assert_eq!(us_reg.preset, "us_regular");
    assert_eq!(us_reg.weekdays.len(), 5);

    let london = build_trading_session("london", "Europe/London", "08:00", "16:30", None);
    assert_eq!(london.preset, "london");

    let asia = build_trading_session("asia", "Asia/Shanghai", "09:00", "16:00", None);
    assert_eq!(asia.preset, "asia");
}
