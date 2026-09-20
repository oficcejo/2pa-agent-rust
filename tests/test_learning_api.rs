//! Router-level tests for the continual-learning endpoints.
//!
//! These assert the routes are actually wired into the protected router (not
//! merely present as handler functions) and that the default posture is safe:
//! the loop is off, and nothing about it is exposed without authentication.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use okx_2pa_agent::config::settings::Settings;
use okx_2pa_agent::web::server::create_router;
use okx_2pa_agent::web::service::WebTradingService;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

const TOKEN: &str = "test-token-abcdefghijklmnopqrst";

fn app() -> axum::Router {
    let settings = Settings {
        web_auth_token: TOKEN.to_string(),
        ..Default::default()
    };
    create_router(Arc::new(WebTradingService::new(settings)))
}

fn get(uri: &str, authorized: bool) -> Request<Body> {
    let builder = Request::builder().uri(uri);
    let builder = if authorized {
        builder.header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
    } else {
        builder
    };
    builder.body(Body::empty()).unwrap()
}

fn post_json(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn learning_routes_all_require_authentication() {
    for uri in [
        "/api/learning/report",
        "/api/learning/outcomes",
        "/api/learning/prompts",
    ] {
        let response = app().oneshot(get(uri, false)).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{uri} must not be reachable without credentials"
        );
    }
}

#[tokio::test]
async fn learning_report_is_served_and_defaults_to_disabled() {
    let response = app().oneshot(get("/api/learning/report", true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;

    assert_eq!(body["enabled"], false, "loop must be opt-in");
    assert_eq!(body["read_experience"], false, "experience injection stays off by default");
    assert_eq!(body["write_experience"], true);
    assert_eq!(body["outcomes"]["total"], 0);
    assert!(body["active_prompt"]["version"].is_string());
    assert!(body["active_prompt"]["hash"].is_string());
}

#[tokio::test]
async fn outcomes_endpoint_returns_an_array() {
    let response = app().oneshot(get("/api/learning/outcomes?limit=5", true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(json_body(response).await.is_array());
}

#[tokio::test]
async fn prompt_versions_seed_and_expose_the_active_revision() {
    let response = app().oneshot(get("/api/learning/prompts", true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;

    let active = body["active"].as_str().unwrap_or_default();
    assert!(!active.is_empty(), "artifact store must seed an active version");
    let versions = body["versions"].as_array().expect("versions array");
    assert!(versions.iter().any(|v| v["version"].as_str() == Some(active)));
}

#[tokio::test]
async fn reconcile_is_gated_on_the_master_switch() {
    let response = app()
        .oneshot(post_json("/api/learning/reconcile", "{}"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let text = String::from_utf8(
        axum::body::to_bytes(response.into_body(), 1 << 16).await.unwrap().to_vec(),
    )
    .unwrap();
    assert!(text.contains("学习闭环未启用"), "{text}");
}

#[tokio::test]
async fn publishing_a_too_short_candidate_is_rejected() {
    let response = app()
        .oneshot(post_json(
            "/api/learning/prompts/publish",
            &serde_json::json!({"content": "too short", "note": "x"}).to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn activating_an_unknown_version_is_rejected() {
    let response = app()
        .oneshot(post_json(
            "/api/learning/prompts/activate",
            &serde_json::json!({"version": "v999", "force": true}).to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn write_endpoints_reject_non_json_requests() {
    // Guards the CSRF posture added with the learning endpoints: a write with
    // no JSON content type must not reach the handler.
    let request = Request::builder()
        .method("POST")
        .uri("/api/learning/reconcile")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let response = app().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn circuit_breaker_reset_and_status_metrics_work() {
    let router = app();
    let res = router.clone().oneshot(get("/api/status", true)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let status_body = json_body(res).await;
    assert_eq!(status_body["circuit_breaker"]["tripped"], false);
    assert_eq!(status_body["circuit_breaker"]["max_loss_usd"], 500.0);
    assert_eq!(status_body["shadow_trading"]["enabled"], false);

    let reset_res = router.oneshot(post_json("/api/learning/circuit_breaker/reset", "{}")).await.unwrap();
    assert_eq!(reset_res.status(), StatusCode::OK);
    let reset_body = json_body(reset_res).await;
    assert_eq!(reset_body["reset"], true);
    assert_eq!(reset_body["tripped"], false);
}

#[tokio::test]
async fn benchmark_episodes_endpoint_returns_list() {
    let response = app().oneshot(get("/api/learning/benchmark_episodes", true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(body.is_array());
    let episodes = body.as_array().unwrap();
    assert!(episodes.len() >= 2, "Seeded benchmark suites should contain episodes");
}

#[tokio::test]
async fn shadow_trading_toggle_endpoint_works() {
    let router = app();
    let res1 = router.clone().oneshot(post_json("/api/learning/shadow_trading/toggle", "{}")).await.unwrap();
    assert_eq!(res1.status(), StatusCode::OK);
    let body1 = json_body(res1).await;
    assert_eq!(body1["enabled"], true);

    let res2 = router.oneshot(post_json("/api/learning/shadow_trading/toggle", "{}")).await.unwrap();
    assert_eq!(res2.status(), StatusCode::OK);
    let body2 = json_body(res2).await;
    assert_eq!(body2["enabled"], false);
}

#[tokio::test]
async fn propose_endpoint_errors_when_no_failures() {
    let response = app().oneshot(post_json("/api/learning/propose", "{}")).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let text = String::from_utf8(
        axum::body::to_bytes(response.into_body(), 1 << 16).await.unwrap().to_vec(),
    ).unwrap();
    assert!(text.contains("失败交易样本") || text.contains("无可用于生成反思突变"), "{text}");
}

#[tokio::test]
async fn solidify_episode_endpoint_errors_for_unknown_signal() {
    let response = app().oneshot(post_json(
        "/api/learning/solidify_episode",
        &serde_json::json!({"signal_id": "non_existent_signal_12345"}).to_string(),
    )).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let text = String::from_utf8(
        axum::body::to_bytes(response.into_body(), 1 << 16).await.unwrap().to_vec(),
    ).unwrap();
    assert!(text.contains("未找到交易结果"), "{text}");
}

#[tokio::test]
async fn learning_report_contains_multidimensional_metrics() {
    let response = app().oneshot(get("/api/learning/report", true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;

    assert!(body["multidimensional"].is_object(), "report must include multidimensional metrics");
    assert!(body["multidimensional"]["overall"].is_object());
    assert!(body["multidimensional"]["by_strategy"].is_object());
    assert!(body["multidimensional"]["by_symbol"].is_object());
}

