use super::service::WebTradingService;
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::{io::Write, path::Path, sync::Arc};

/// Persist a generated password once; never put it in HTTP responses or logs.
pub fn load_or_create_token(path: &Path) -> anyhow::Result<String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let token = std::fs::read_to_string(path)?.trim().to_string();
        anyhow::ensure!(token.len() >= 24, "Web 登录口令文件无效");
        return Ok(token);
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(token.as_bytes())?;
    file.sync_all()?;
    Ok(token)
}

pub async fn authenticate(
    State(service): State<Arc<WebTradingService>>,
    req: Request,
    next: Next,
) -> Response {
    let expected = service.settings.read().web_auth_token.clone();
    if expected.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Web 登录未配置，请设置 WEB_AUTH_TOKEN 后重启",
        )
            .into_response();
    }
    let auth = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let supplied = if let Some(token) = auth.strip_prefix("Bearer ") {
        Some(token.to_string())
    } else if let Some(encoded) = auth.strip_prefix("Basic ") {
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|v| v.strip_prefix("admin:").map(str::to_string))
    } else {
        None
    };
    let valid = supplied
        .map(|token| {
            let a = Sha256::digest(token.as_bytes());
            let b = Sha256::digest(expected.as_bytes());
            a.iter()
                .zip(b.iter())
                .fold(0u8, |diff, (x, y)| diff | (x ^ y))
                == 0
        })
        .unwrap_or(false);
    if !valid {
        return (
            StatusCode::UNAUTHORIZED,
            [
                (
                    header::WWW_AUTHENTICATE,
                    "Basic realm=\"2PA\", charset=\"UTF-8\"",
                ),
                (header::CACHE_CONTROL, "no-store"),
            ],
            "请使用 admin 和站点口令登录",
        )
            .into_response();
    }
    if !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD
    ) && (req
        .headers()
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        == Some("cross-site")
        || !req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or("").trim() == "application/json")
            .unwrap_or(false))
    {
        return (StatusCode::FORBIDDEN, "拒绝跨站写请求或非 JSON 请求").into_response();
    }
    let mut response = next.run(req).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
