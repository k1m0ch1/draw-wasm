//! REST API. Every handler validates its input, rate-limits by hashed client address and returns
//! short generic error messages (no internals).

use crate::db;
use crate::limits::{ct_eq, hash_ip, sha256_hex};
use crate::state::{lock, nanoid, App};
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use proto::Element;
use rand::RngCore;
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;

pub type Res = Result<Response, Response>;

pub fn err(code: StatusCode, msg: &str) -> Response {
    (code, [(header::CACHE_CONTROL, "no-store")], Json(json!({ "error": msg }))).into_response()
}
fn ok(v: serde_json::Value) -> Res {
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(v)).into_response())
}
fn internal() -> Response {
    err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

/// Hashed client identity used only for rate limiting. The reverse proxy sets X-Real-IP; it is only
/// trusted when the TCP peer is a private/loopback address (i.e. the proxy itself).
pub fn client_key(app: &App, peer: &SocketAddr, h: &HeaderMap) -> String {
    let mut ip = peer.ip().to_string();
    let private = match peer.ip() {
        std::net::IpAddr::V4(v) => v.is_loopback() || v.is_private() || v.is_link_local(),
        std::net::IpAddr::V6(v) => v.is_loopback() || (v.segments()[0] & 0xfe00) == 0xfc00,
    };
    if private {
        if let Some(x) = h.get("x-real-ip").and_then(|v| v.to_str().ok()) {
            if x.len() <= 45 && x.bytes().all(|b| b.is_ascii_hexdigit() || b == b'.' || b == b':') {
                ip = x.to_string();
            }
        }
    }
    hash_ip(&app.salt, &ip)
}

fn rate(app: &App, key: &str, bucket: &str, cap: f64, per_sec: f64) -> Result<(), Response> {
    if app.limiter.check(&format!("{bucket}:{key}"), cap, per_sec) {
        Ok(())
    } else {
        Err((
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "5"), (header::CACHE_CONTROL, "no-store")],
            Json(json!({ "error": "too many requests" })),
        )
            .into_response())
    }
}

fn general(app: &App, peer: &SocketAddr, h: &HeaderMap) -> Result<String, Response> {
    let k = client_key(app, peer, h);
    rate(app, &k, "api", 60.0, 10.0)?;
    Ok(k)
}

fn owner_hash(h: &HeaderMap) -> Option<String> {
    let t = h.get("x-owner-token")?.to_str().ok()?;
    if t.len() < 20 || t.len() > 128 || !t.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    Some(sha256_hex(t.as_bytes()))
}

async fn is_owner(app: &App, room: &str, h: &HeaderMap) -> Result<bool, Response> {
    let Some(hash) = owner_hash(h) else { return Ok(false) };
    let r = room.to_string();
    let stored: Option<Option<String>> = app
        .db
        .call(move |c| {
            c.query_row("SELECT owner_hash FROM rooms WHERE id=?1", params![r], |row| row.get::<_, String>(0)).ok()
        })
        .await;
    match stored {
        None => Err(internal()),
        Some(None) => Ok(false),
        Some(Some(s)) => Ok(ct_eq(&s, &hash)),
    }
}

fn base_url(app: &App, h: &HeaderMap) -> String {
    if let Some(b) = &app.cfg.public_base_url {
        return b.trim_end_matches('/').to_string();
    }
    let host = h
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .filter(|s| s.len() <= 100 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b':'))
        .unwrap_or("localhost");
    let proto = match h.get("x-forwarded-proto").and_then(|v| v.to_str().ok()) {
        Some("https") => "https",
        _ => "http",
    };
    format!("{proto}://{host}")
}

pub async fn healthz(State(app): State<Arc<App>>) -> Response {
    match app.db.call(|c| c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0)).is_ok()).await {
        Some(true) => ([(header::CACHE_CONTROL, "no-store")], "ok").into_response(),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "unavailable").into_response(),
    }
}

pub async fn not_found() -> Response {
    err(StatusCode::NOT_FOUND, "not found")
}

#[derive(Deserialize)]
pub struct ClaimReq {
    room: String,
}

fn bad_room(room: &str) -> Option<Response> {
    proto::valid_room(room).err().map(|m| err(StatusCode::UNPROCESSABLE_ENTITY, m))
}

/// First come, first served. The returned token is shown once; only its hash is kept.
pub async fn claim_room(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    h: HeaderMap,
    body: Bytes,
) -> Res {
    let k = general(&app, &peer, &h)?;
    rate(&app, &k, "claim", 5.0, 0.05)?;
    let req: ClaimReq = serde_json::from_slice(&body).map_err(|_| err(StatusCode::BAD_REQUEST, "invalid request"))?;
    if let Some(e) = bad_room(&req.room) {
        return Err(e);
    }
    let mut raw = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut raw);
    let token: String = raw.iter().map(|b| format!("{:02x}", b)).collect();
    let hash = sha256_hex(token.as_bytes());
    let (room, max) = (req.room.clone(), app.cfg.max_rooms);
    let res = app
        .db
        .call(move |c| {
            let n: i64 = c.query_row("SELECT count(*) FROM rooms", [], |r| r.get(0)).unwrap_or(0);
            if n >= max {
                return 2;
            }
            match c.execute(
                "INSERT OR IGNORE INTO rooms (id, owner_hash, created) VALUES (?1, ?2, ?3)",
                params![room, hash, db::now()],
            ) {
                Ok(1) => 0,
                Ok(_) => 1,
                Err(_) => 3,
            }
        })
        .await
        .unwrap_or(3);
    match res {
        0 => ok(json!({ "room": req.room, "token": token })),
        1 => Err(err(StatusCode::CONFLICT, "this room id is already taken")),
        2 => Err(err(StatusCode::TOO_MANY_REQUESTS, "server is at capacity")),
        _ => Err(internal()),
    }
}

pub async fn get_room(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(room): Path<String>,
    h: HeaderMap,
) -> Res {
    general(&app, &peer, &h)?;
    if let Some(e) = bad_room(&room) {
        return Err(e);
    }
    let owner = is_owner(&app, &room, &h).await?;
    let r = room.clone();
    let rows: Option<Option<Vec<(String, String, i64)>>> = app
        .db
        .call(move |c| {
            let exists: bool = c.query_row("SELECT 1 FROM rooms WHERE id=?1", params![r], |_| Ok(())).is_ok();
            if !exists {
                return None;
            }
            let mut st = c.prepare("SELECT nano, title, updated FROM docs WHERE room=?1 ORDER BY updated DESC LIMIT 200").ok()?;
            let v = st
                .query_map(params![r], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .ok()?
                .flatten()
                .collect();
            Some(v)
        })
        .await;
    match rows {
        None => Err(internal()),
        Some(None) => Err(err(StatusCode::NOT_FOUND, "room not found")),
        Some(Some(v)) => {
            let docs: Vec<_> = v.into_iter().map(|(n, t, u)| json!({ "nano": n, "title": t, "updated": u })).collect();
            ok(json!({ "room": room, "owner": owner, "docs": docs }))
        }
    }
}

pub async fn delete_room(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(room): Path<String>,
    h: HeaderMap,
) -> Res {
    let k = general(&app, &peer, &h)?;
    rate(&app, &k, "del", 10.0, 0.2)?;
    if let Some(e) = bad_room(&room) {
        return Err(e);
    }
    if !is_owner(&app, &room, &h).await? {
        return Err(err(StatusCode::FORBIDDEN, "only the room owner can do this"));
    }
    let r = room.clone();
    let nanos: Vec<String> = app
        .db
        .call(move |c| {
            let mut st = c.prepare("SELECT nano FROM docs WHERE room=?1").ok()?;
            let v: Vec<String> = st.query_map(params![r], |row| row.get(0)).ok()?.flatten().collect();
            Some(v)
        })
        .await
        .flatten()
        .unwrap_or_default();
    for n in &nanos {
        app.drop_doc(n).await;
    }
    let r = room.clone();
    app.db
        .call(move |c| {
            let _ = c.execute("DELETE FROM docs WHERE room=?1", params![r]);
            let _ = c.execute("DELETE FROM rooms WHERE id=?1", params![r]);
            db::gc_images(c);
        })
        .await;
    ok(json!({ "ok": true }))
}

#[derive(Deserialize)]
pub struct CreateDoc {
    #[serde(default)]
    title: String,
    #[serde(default)]
    els: Vec<Element>,
}

pub fn clean_title(s: &str) -> String {
    let t: String = s.chars().filter(|c| !c.is_control()).take(80).collect();
    let t = t.trim().to_string();
    if t.is_empty() {
        "Untitled".into()
    } else {
        t
    }
}

/// Validate a set of elements against the document limits. Returns the serialized snapshot.
pub fn check_elements(app: &App, els: &[Element]) -> Result<String, Response> {
    if els.len() > app.cfg.max_elements {
        return Err(err(StatusCode::PAYLOAD_TOO_LARGE, "too many elements"));
    }
    let mut seen = std::collections::HashSet::new();
    for e in els {
        if let Err(m) = e.validate() {
            return Err(err(StatusCode::UNPROCESSABLE_ENTITY, m));
        }
        if !seen.insert(&e.id) {
            return Err(err(StatusCode::UNPROCESSABLE_ENTITY, "duplicate element id"));
        }
    }
    let snap = serde_json::to_string(els).map_err(|_| internal())?;
    if snap.len() > app.cfg.max_doc_bytes {
        return Err(err(StatusCode::PAYLOAD_TOO_LARGE, "document too large"));
    }
    Ok(snap)
}

pub async fn create_doc(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(room): Path<String>,
    h: HeaderMap,
    body: Bytes,
) -> Res {
    let k = general(&app, &peer, &h)?;
    rate(&app, &k, "mkdoc", 10.0, 0.1)?;
    if let Some(e) = bad_room(&room) {
        return Err(e);
    }
    if body.len() > app.cfg.max_doc_bytes + 4096 {
        return Err(err(StatusCode::PAYLOAD_TOO_LARGE, "document too large"));
    }
    let req: CreateDoc = serde_json::from_slice(&body).map_err(|_| err(StatusCode::BAD_REQUEST, "invalid request"))?;
    let snap = check_elements(&app, &req.els)?;
    let title = clean_title(&req.title);
    let (r, max) = (room.clone(), app.cfg.max_docs_per_room);
    let res: Option<Result<String, u8>> = app
        .db
        .call(move |c| {
            let exists: bool = c.query_row("SELECT 1 FROM rooms WHERE id=?1", params![r], |_| Ok(())).is_ok();
            if !exists {
                return Err(1u8);
            }
            let n: i64 = c.query_row("SELECT count(*) FROM docs WHERE room=?1", params![r], |row| row.get(0)).unwrap_or(0);
            if n >= max {
                return Err(2);
            }
            // NanoIDs are unique per database: retry on collision
            for _ in 0..8 {
                let id = nanoid();
                let t = db::now();
                match c.execute(
                    "INSERT OR IGNORE INTO docs (nano, room, title, snap, seq, created, updated) VALUES (?1,?2,?3,?4,1,?5,?5)",
                    params![id, r, title, snap, t],
                ) {
                    Ok(1) => return Ok(id),
                    Ok(_) => continue,
                    Err(_) => return Err(3),
                }
            }
            Err(3)
        })
        .await;
    match res {
        Some(Ok(nano)) => {
            let url = format!("{}/{}/{}", base_url(&app, &h), room, nano);
            ok(json!({ "nano": nano, "url": url }))
        }
        Some(Err(1)) => Err(err(StatusCode::NOT_FOUND, "room not found")),
        Some(Err(2)) => Err(err(StatusCode::TOO_MANY_REQUESTS, "this room has too many documents")),
        _ => Err(internal()),
    }
}

pub async fn get_doc(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path((room, nano)): Path<(String, String)>,
    h: HeaderMap,
) -> Res {
    general(&app, &peer, &h)?;
    if let Some(e) = bad_room(&room) {
        return Err(e);
    }
    if !proto::valid_nano(&nano) {
        return Err(err(StatusCode::NOT_FOUND, "document not found"));
    }
    // prefer the live in-memory copy
    if let Some(a) = app.docs.lock().await.get(&nano).cloned() {
        if a.room == room {
            let g = lock(&a.inner);
            let els: Vec<&Element> = g.doc.els.values().collect();
            return ok(json!({ "nano": nano, "room": room, "title": g.title, "seq": g.seq, "els": els }));
        }
    }
    let (r, n) = (room.clone(), nano.clone());
    let row: Option<Option<(String, String, i64)>> = app
        .db
        .call(move |c| {
            c.query_row("SELECT title, snap, seq FROM docs WHERE nano=?1 AND room=?2", params![n, r], |x| {
                Ok((x.get(0)?, x.get(1)?, x.get(2)?))
            })
            .ok()
        })
        .await;
    match row {
        None => Err(internal()),
        Some(None) => Err(err(StatusCode::NOT_FOUND, "document not found")),
        Some(Some((title, snap, seq))) => {
            let els: serde_json::Value = serde_json::from_str(&snap).unwrap_or(json!([]));
            ok(json!({ "nano": nano, "room": room, "title": title, "seq": seq, "els": els }))
        }
    }
}

pub async fn delete_doc(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path((room, nano)): Path<(String, String)>,
    h: HeaderMap,
) -> Res {
    let k = general(&app, &peer, &h)?;
    rate(&app, &k, "del", 10.0, 0.2)?;
    if let Some(e) = bad_room(&room) {
        return Err(e);
    }
    if !proto::valid_nano(&nano) {
        return Err(err(StatusCode::NOT_FOUND, "document not found"));
    }
    if !is_owner(&app, &room, &h).await? {
        return Err(err(StatusCode::FORBIDDEN, "only the room owner can do this"));
    }
    app.drop_doc(&nano).await;
    let (r, n) = (room, nano);
    let deleted = app
        .db
        .call(move |c| {
            let d = c.execute("DELETE FROM docs WHERE nano=?1 AND room=?2", params![n, r]).unwrap_or(0);
            db::gc_images(c);
            d
        })
        .await
        .unwrap_or(0);
    if deleted == 0 {
        return Err(err(StatusCode::NOT_FOUND, "document not found"));
    }
    ok(json!({ "ok": true }))
}

/// Detect the real image type from the first bytes. SVG is deliberately not supported.
pub fn sniff(b: &[u8]) -> Option<&'static str> {
    if b.len() >= 8 && b[..8] == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
        Some("image/png")
    } else if b.len() >= 3 && b[..3] == [0xff, 0xd8, 0xff] {
        Some("image/jpeg")
    } else if b.len() >= 6 && (&b[..6] == b"GIF87a" || &b[..6] == b"GIF89a") {
        Some("image/gif")
    } else if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

pub async fn upload_image(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path((room, nano)): Path<(String, String)>,
    h: HeaderMap,
    body: Bytes,
) -> Res {
    let k = general(&app, &peer, &h)?;
    rate(&app, &k, "img", 30.0, 1.0)?;
    if let Some(e) = bad_room(&room) {
        return Err(e);
    }
    if !proto::valid_nano(&nano) {
        return Err(err(StatusCode::NOT_FOUND, "document not found"));
    }
    if body.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "empty body"));
    }
    if body.len() > app.cfg.max_image_bytes {
        return Err(err(StatusCode::PAYLOAD_TOO_LARGE, "image too large"));
    }
    let Some(mime) = sniff(&body) else {
        return Err(err(StatusCode::UNSUPPORTED_MEDIA_TYPE, "not a supported image (png, jpeg, gif, webp)"));
    };
    let sha = sha256_hex(&body);
    let (r, n, s, data) = (room, nano, sha.clone(), body.to_vec());
    let (max_n, max_b) = (app.cfg.max_doc_images, app.cfg.max_doc_image_bytes);
    let size = data.len() as i64;
    let res: Option<u8> = app
        .db
        .call(move |c| {
            let tx = match c.transaction() {
                Ok(t) => t,
                Err(_) => return 3,
            };
            let exists: bool = tx
                .query_row("SELECT 1 FROM docs WHERE nano=?1 AND room=?2", params![n, r], |_| Ok(()))
                .is_ok();
            if !exists {
                return 1;
            }
            let linked: bool = tx
                .query_row("SELECT 1 FROM doc_images WHERE doc=?1 AND sha=?2", params![n, s], |_| Ok(()))
                .is_ok();
            if !linked {
                let (cnt, total): (i64, i64) = tx
                    .query_row(
                        "SELECT count(*), coalesce(sum(i.size),0) FROM doc_images d JOIN images i ON i.sha=d.sha WHERE d.doc=?1",
                        params![n],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap_or((0, 0));
                if cnt >= max_n || total + size > max_b {
                    return 2;
                }
                if tx
                    .execute(
                        "INSERT OR IGNORE INTO images (sha, mime, size, bytes, created) VALUES (?1,?2,?3,?4,?5)",
                        params![s, mime, size, data, db::now()],
                    )
                    .is_err()
                    || tx.execute("INSERT OR IGNORE INTO doc_images (doc, sha) VALUES (?1,?2)", params![n, s]).is_err()
                {
                    return 3;
                }
            }
            if tx.commit().is_err() {
                return 3;
            }
            0
        })
        .await;
    match res {
        Some(0) => ok(json!({ "sha": sha })),
        Some(1) => Err(err(StatusCode::NOT_FOUND, "document not found")),
        Some(2) => Err(err(StatusCode::PAYLOAD_TOO_LARGE, "image limit for this document reached")),
        _ => Err(internal()),
    }
}

pub async fn get_image(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(sha): Path<String>,
    h: HeaderMap,
) -> Res {
    let k = client_key(&app, &peer, &h);
    rate(&app, &k, "imgget", 300.0, 50.0)?;
    if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(err(StatusCode::NOT_FOUND, "not found"));
    }
    let s = sha.clone();
    let row: Option<Option<(String, Vec<u8>)>> = app
        .db
        .call(move |c| c.query_row("SELECT mime, bytes FROM images WHERE sha=?1", params![s], |r| Ok((r.get(0)?, r.get(1)?))).ok())
        .await;
    match row {
        None => Err(internal()),
        Some(None) => Err(err(StatusCode::NOT_FOUND, "not found")),
        Some(Some((mime, bytes))) => Ok((
            [
                (header::CONTENT_TYPE, mime),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable".to_string()),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
                (header::CONTENT_SECURITY_POLICY, "default-src 'none'; sandbox".to_string()),
            ],
            bytes,
        )
            .into_response()),
    }
}
