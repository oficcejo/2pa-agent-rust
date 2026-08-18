use crate::config::settings::Settings;
use crate::web::handlers::*;
use crate::web::service::WebTradingService;
use anyhow::Result;
use axum::routing::{delete, get, post};
use axum::Router;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tracing::info;

pub fn create_router(service: Arc<WebTradingService>) -> Router {
    Router::new()
        .route("/", get(handle_index))
        .route("/static/*path", get(handle_static))
        .route("/api/status", get(handle_status))
        .route("/api/instruments", get(handle_instruments))
        .route("/api/candles", get(handle_candles))
        .route("/api/account", get(handle_account))
        .route("/api/history/decisions", get(handle_get_decision_history))
        .route("/api/history/decisions/:record_id", delete(handle_delete_decision_history))
        .route("/api/history/trades", get(handle_get_trade_history))
        .route("/api/history/trades/:record_id", delete(handle_delete_trade_history))
        .route("/api/analyze", post(handle_analyze))
        .route("/api/automation", post(handle_automation))
        .route("/api/config", get(handle_get_config))
        .route("/api/config/save_env", post(handle_save_config))
        .route("/api/trading_system", post(handle_set_trading_system))
        .route("/api/contract/specs", get(handle_contract_specs))
        .layer(CorsLayer::permissive())
        .with_state(service)
}

pub async fn run_server(host: &str, port: u16, settings: Settings) -> Result<()> {
    let service = Arc::new(WebTradingService::new(settings.clone()));

    // Spawn background automation tick loop
    let poll_seconds = settings.okx.automation_poll_seconds.max(5);
    let auto_service = Arc::clone(&service);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(poll_seconds));
        loop {
            interval.tick().await;
            if let Err(e) = auto_service.automation_tick().await {
                tracing::warn!("Automation loop error: {}", e);
            }
        }
    });

    let app = create_router(service);
    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    info!("Starting okx-2pa-agent server on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
