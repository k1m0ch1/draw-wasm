//! Static files, held in memory (the bundle is small), plus the SPA entry for room/doc URLs.

use crate::state::App;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::sync::Arc;

pub struct File {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}
pub type Statics = HashMap<String, File>;

fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "json" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        "png" => "image/png",
        _ => "application/octet-stream",
    }
}

fn walk(base: &std::path::Path, dir: &std::path::Path, out: &mut Statics) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for ent in rd.flatten() {
        let p = ent.path();
        if p.is_dir() {
            walk(base, &p, out);
        } else if let (Ok(rel), Ok(bytes)) = (p.strip_prefix(base), std::fs::read(&p)) {
            let key = rel.to_string_lossy().replace('\\', "/");
            out.insert(key.clone(), File { mime: mime_for(&key), bytes });
        }
    }
}

pub fn load(dir: &str) -> Statics {
    let mut m = HashMap::new();
    let base = std::path::Path::new(dir);
    walk(base, base, &mut m);
    println!("static files loaded: {}", m.len());
    m
}

fn respond(f: &File, cache: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, f.mime),
            (header::CACHE_CONTROL, cache),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        f.bytes.clone(),
    )
        .into_response()
}

pub async fn asset(State(app): State<Arc<App>>, Path(path): Path<String>) -> Response {
    if path.contains("..") || path.contains('\\') || path.starts_with('/') {
        return StatusCode::NOT_FOUND.into_response();
    }
    match app.statics.get(&format!("assets/{path}")) {
        // file names carry a content hash, so they can be cached forever
        Some(f) => respond(f, "public, max-age=31536000, immutable"),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn index_resp(app: &App) -> Response {
    match app.statics.get("index.html") {
        Some(f) => respond(f, "no-cache"),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn index(State(app): State<Arc<App>>) -> Response {
    index_resp(&app)
}

pub async fn room_page(State(app): State<Arc<App>>, Path(room): Path<String>) -> Response {
    if proto::valid_room(&room).is_err() {
        return StatusCode::NOT_FOUND.into_response();
    }
    index_resp(&app)
}

pub async fn doc_page(State(app): State<Arc<App>>, Path((room, nano)): Path<(String, String)>) -> Response {
    if proto::valid_room(&room).is_err() || !proto::valid_nano(&nano) {
        return StatusCode::NOT_FOUND.into_response();
    }
    index_resp(&app)
}

pub async fn favicon(State(app): State<Arc<App>>) -> Response {
    match app.statics.get("favicon.svg") {
        Some(f) => respond(f, "public, max-age=86400"),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn robots(State(app): State<Arc<App>>) -> Response {
    match app.statics.get("robots.txt") {
        Some(f) => respond(f, "public, max-age=86400"),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
