#![allow(dead_code)]
use axum::{
    extract::{Request, State},
    response::IntoResponse,
    Json, Router,
};
use okx_2pa_agent::okx::client::{OKXClient, OKXCredentials};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
struct MockState {
    routes: Arc<HashMap<String, Value>>,
    calls: Arc<Mutex<Vec<(String, String, Value)>>>,
}

async fn respond(State(state): State<MockState>, request: Request) -> axum::response::Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let query = request.uri().query().unwrap_or("").to_string();
    let bytes = axum::body::to_bytes(request.into_body(), 65536)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    state
        .calls
        .lock()
        .unwrap()
        .push((method, format!("{path}?{query}"), body));
    match state
        .routes
        .get(&format!("{path}?{query}"))
        .or_else(|| state.routes.get(&path))
    {
        Some(v) => {
            if v.get("_http_status").and_then(|s| s.as_u64()) == Some(403) {
                let mut body = v.clone();
                if let Some(obj) = body.as_object_mut() { obj.remove("_http_status"); }
                return (axum::http::StatusCode::FORBIDDEN, Json(body)).into_response();
            }
            Json(v.clone()).into_response()
        }
        None => (axum::http::StatusCode::NOT_FOUND, "unexpected mock request").into_response(),
    }
}

pub struct Mock {
    pub client: OKXClient,
    pub url: String,
    pub calls: Arc<Mutex<Vec<(String, String, Value)>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Mock {
    pub async fn start(routes: HashMap<String, Value>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let calls = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new().fallback(respond).with_state(MockState {
            routes: Arc::new(routes),
            calls: calls.clone(),
        });
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = OKXClient::new(
            &url,
            Some(OKXCredentials::new("test", "test", "test")),
            true,
            5,
        );
        Self {
            client,
            url,
            calls,
            task,
        }
    }
}
pub fn ok(data: Value) -> Value {
    json!({"code":"0", "data":data})
}
pub fn routes() -> HashMap<String, Value> {
    HashMap::from([
        (
            "/api/v5/public/instruments".into(),
            ok(
                json!([{"instId":"TEST-USDT-SWAP", "tickSz":"0.01", "lotSz":"0.01", "minSz":"0.01", "ctVal":"1", "ctType":"linear", "settleCcy":"USDT", "quoteCcy":"USDT"}]),
            ),
        ),
        (
            "/api/v5/account/balance".into(),
            ok(
                json!([{"totalEq":"1000", "details":[{"ccy":"USDT", "availEq":"1000", "availBal":"1000"}]}]),
            ),
        ),
        ("/api/v5/account/positions".into(), ok(json!([]))),
        ("/api/v5/trade/orders-pending".into(), ok(json!([]))),
        ("/api/v5/trade/orders-algo-pending".into(), ok(json!([]))),
        (
            "/api/v5/account/set-leverage".into(),
            ok(json!([{"lever":"3"}])),
        ),
        (
            "/api/v5/trade/order".into(),
            ok(json!([{"sCode":"0", "ordId":"new-order"}])),
        ),
        ("/api/v5/market/ticker".into(), ok(json!([{"last":"100"}]))),
    ])
}
