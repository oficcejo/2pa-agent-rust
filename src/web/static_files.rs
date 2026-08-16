use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::RustEmbed;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

#[derive(RustEmbed)]
#[folder = "static/"]
pub struct StaticAssets;

pub fn asset_version() -> String {
    let mut hasher = Sha256::new();
    for fname in ["index.html", "app.js", "app.css"] {
        hasher.update(fname.as_bytes());
        if let Some(file) = StaticAssets::get(fname) {
            hasher.update(&file.data);
        }
    }
    let hex = hex::encode(hasher.finalize());
    hex[..12].to_string()
}

pub fn render_index() -> Html<String> {
    let raw = match StaticAssets::get("index.html") {
        Some(f) => String::from_utf8_lossy(&f.data).to_string(),
        None => "<h1>okx-2pa-agent-web</h1>".to_string(),
    };
    let version = asset_version();
    Html(raw.replace("__ASSET_VERSION__", &version))
}

pub fn get_static_asset(path: &str) -> Response {
    let clean_path = path.trim_start_matches('/');

    // 1. Try local filesystem if static directory exists
    let disk_path = Path::new("static").join(clean_path);
    if disk_path.is_file() {
        if let Ok(bytes) = fs::read(&disk_path) {
            return serve_bytes(&bytes, clean_path);
        }
    }

    // 2. Embedded asset
    if let Some(asset) = StaticAssets::get(clean_path) {
        return serve_bytes(&asset.data, clean_path);
    }

    (StatusCode::NOT_FOUND, "File Not Found").into_response()
}

fn serve_bytes(bytes: &[u8], path: &str) -> Response {
    let mime = if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".js") {
        "application/javascript; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".json") {
        "application/json"
    } else {
        "application/octet-stream"
    };

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));

    (headers, bytes.to_vec()).into_response()
}
