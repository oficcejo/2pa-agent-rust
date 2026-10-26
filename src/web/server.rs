use crate::config::settings::Settings;
use crate::web::handlers::*;
use crate::web::service::WebTradingService;
use anyhow::Result;
use axum::routing::{delete, get, post};
use axum::Router;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

/// Wildcard and loopback bindings are the only addresses where serving an
/// unauthenticated trading console is acceptable.
fn is_loopback_host(host: &str) -> bool {
    let h = host.trim();
    h == "127.0.0.1" || h == "localhost" || h == "::1" || h == "[::1]" || h.starts_with("127.")
}

pub fn create_router(service: Arc<WebTradingService>) -> Router {
    let app = Router::new()
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
        .route("/api/trade/cancel", post(handle_cancel_order))
        .route("/api/trade/cancel_all", post(handle_cancel_all_orders))
        .route("/api/learning/report", get(handle_learning_report))
        .route("/api/learning/reconcile", post(handle_learning_reconcile))
        .route("/api/learning/outcomes", get(handle_learning_outcomes))
        .route("/api/learning/prompts", get(handle_prompt_versions))
        .route("/api/learning/prompts/publish", post(handle_publish_prompt))
        .route("/api/learning/prompts/activate", post(handle_activate_prompt))
        .route("/api/learning/prompts/rollback", post(handle_rollback_prompt))
        .route("/api/learning/propose", post(handle_learning_propose))
        .route("/api/learning/solidify_episode", post(handle_solidify_episode))
        .route("/api/learning/benchmark_episodes", get(handle_list_benchmark_episodes))
        .route("/api/learning/circuit_breaker/reset", post(handle_reset_circuit_breaker))
        .route("/api/learning/shadow_trading/toggle", post(handle_toggle_shadow_trading))
        .route("/api/backtest/run", post(handle_run_backtest))
        .route("/api/backtest/status/:id", get(handle_backtest_status))
        .route("/api/backtest/report/:id", get(handle_backtest_report));

    // 鉴权开关：默认关闭（web_auth_enabled = false）时，全部路由免登录直接可达。
    // 开启后才挂上 authenticate 中间件（Bearer/Basic 校验 + 跨站写拦截）。
    if service.settings.read().web_auth_enabled {
        app.layer(axum::middleware::from_fn_with_state(
            service.clone(),
            crate::web::auth::authenticate,
        ))
        .with_state(service)
    } else {
        app.with_state(service)
    }
}

pub async fn run_server(host: &str, port: u16, mut settings: Settings) -> Result<()> {
    if settings.web_auth_enabled {
        if settings.web_auth_token.is_empty() {
            settings.web_auth_token = crate::web::auth::load_or_create_token(std::path::Path::new("config/.web-auth-token"))?;
            info!("Web 登录用户名 admin；口令保存在 config/.web-auth-token，远程访问请使用 HTTPS");
        }
    } else {
        // 鉴权显式关闭：本系统可直接驱动真实下单、改写 OKX 密钥，
        // 非回环监听 + 免登录等于把交易权限暴露给整个网络，拒绝启动。
        let loopback = is_loopback_host(host);
        anyhow::ensure!(
            loopback,
            "安全拒绝启动：WEB_AUTH_ENABLED=false（鉴权关闭）且监听地址 {} 非回环地址。\
             如需远程访问请开启鉴权（移除 WEB_AUTH_ENABLED=false 或设为 true），\
             或改为绑定 127.0.0.1 后经带认证的反向代理对外。",
            host
        );
        info!("Web 登录鉴权已显式关闭（仅本地回环 {}），可直接访问 http://{}:{}", host, host, port);
    }
    let service = Arc::new(WebTradingService::new(settings.clone()));

    // One-shot OKX connectivity probe so a dead proxy / blocked network is
    // reported immediately instead of as opaque 502s later on.
    let probe_service = Arc::clone(&service);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = probe_service.connectivity_probe().await;
    });

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

    // Spawn the outcome reconciliation loop when the learning layer is on.
    if settings.learning.enabled {
        let interval_secs = settings.learning.reconcile_interval_seconds.max(10);
        let learn_service = Arc::clone(&service);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;
                if let Err(e) = learn_service.reconcile_outcomes().await {
                    tracing::warn!("Learning reconciliation error: {}", e);
                }
            }
        });
        info!("持续学习闭环已启用：每 {} 秒对账一次交易结果", interval_secs);
    }

    let app = create_router(service);
    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    info!("Starting okx-2pa-agent server on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
