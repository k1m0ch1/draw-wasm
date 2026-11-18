//! WebSocket endpoint: /ws/<room>/<nano>
//!
//! Protocol (JSON text frames, see the `proto` crate): hello -> snap, then ops/cur/ping/save.
//! Ops are merged with last-writer-wins by (version, writer) and broadcast with a per-document
//! monotonically increasing sequence number.

use crate::api::client_key;
use crate::limits::LocalBucket;
use crate::state::*;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use proto::{ClientMsg, Element, Peer, ServerMsg};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

const MAX_OPS_PER_MSG: usize = 500;
const IDLE_TIMEOUT: Duration = Duration::from_secs(75);

pub async fn ws_handler(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path((room, nano)): Path<(String, String)>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if proto::valid_room(&room).is_err() || !proto::valid_nano(&nano) {
        return StatusCode::NOT_FOUND.into_response();
    }
    // same-origin only: browsers always send Origin on WebSocket handshakes
    if let Some(o) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        let host = headers.get("host").and_then(|v| v.to_str().ok()).unwrap_or("");
        let oh = o.split("://").nth(1).unwrap_or("");
        let configured = app.cfg.public_base_url.as_deref().map(|b| b.split("://").nth(1).unwrap_or("")).unwrap_or("");
        if oh != host && (configured.is_empty() || oh != configured) {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    let key = client_key(&app, &peer, &headers);
    if !app.limiter.check(&format!("wsopen:{key}"), 20.0, 1.0) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    if !app.limiter.conn_inc(&key, app.cfg.max_conns_per_ip) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let max = app.cfg.max_ws_msg;
    ws.max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |sock| async move {
            handle(app.clone(), sock, room, nano, key.clone()).await;
            app.limiter.conn_dec(&key);
        })
}

async fn send_err(sock: &mut WebSocket, msg: &str) {
    let m = encode(&ServerMsg::Err { msg: msg.to_string() });
    let _ = sock.send(Message::Text(m.as_str().into())).await;
}

/// True when `token` is the ownership token of `room` (only its hash is stored and compared).
async fn room_owner(app: &App, room: &str, token: &str) -> bool {
    if token.len() < 20 || token.len() > 128 || !token.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return false;
    }
    let hash = crate::limits::sha256_hex(token.as_bytes());
    let r = room.to_string();
    let stored: Option<Option<String>> = app
        .db
        .call(move |c| {
            c.query_row("SELECT owner_hash FROM rooms WHERE id=?1", rusqlite::params![r], |row| row.get::<_, String>(0)).ok()
        })
        .await;
    matches!(stored, Some(Some(s)) if crate::limits::ct_eq(&s, &hash))
}

async fn handle(app: Arc<App>, mut sock: WebSocket, room: String, nano: String, key: String) {
    // 1. hello (must arrive quickly)
    let hello = tokio::time::timeout(Duration::from_secs(10), sock.recv()).await;
    let (cid, name, token, build) = match hello {
        Ok(Some(Ok(Message::Text(t)))) => match serde_json::from_str::<ClientMsg>(&t) {
            Ok(ClientMsg::Hello { cid, name, token, build, .. }) if proto::valid_id(&cid) => (cid, name, token, build),
            _ => {
                send_err(&mut sock, "expected hello").await;
                return;
            }
        },
        _ => return,
    };
    let is_owner = room_owner(&app, &room, &token).await;
    let name = match clean_name(&name) {
        Some(n) => n,
        None if is_owner => "Owner".to_string(),
        // a page older than the name prompt cannot show a dialog: admit it as a guest (it is told to
        // reload below) instead of rejecting it in a reconnect loop
        None if build.is_empty() => format!("Guest {}", rand::Rng::gen_range(&mut rand::thread_rng(), 1000..10000)),
        None => {
            send_err(&mut sock, "a display name (1 to 32 characters) is required").await;
            return;
        }
    };
    let conn = app.next_conn.fetch_add(1, Ordering::Relaxed);
    let (tx, mut rx) = mpsc::channel::<Arc<String>>(512);
    let peer = Peer { id: cid.clone(), name, color: random_color(), owner: is_owner };
    let Some((active, snap)) = app.join(&room, &nano, conn, Client { peer, tx: tx.clone() }).await else {
        send_err(&mut sock, "document not found").await;
        return;
    };
    let first = encode(&snap);
    if sock.send(Message::Text(first.as_str().into())).await.is_err() {
        app.leave(&active, conn).await;
        return;
    }

    if build != app.build && !app.build.is_empty() {
        let _ = sock.send(Message::Text(encode(&ServerMsg::Reload { build: app.build.clone() }).as_str().into())).await;
    }

    let mut ops_bucket = LocalBucket::new(60.0, 30.0);
    let mut cur_bucket = LocalBucket::new(40.0, 30.0);
    let mut violations = 0u32;
    let _ = &key;

    loop {
        tokio::select! {
            out = rx.recv() => {
                match out {
                    Some(m) => {
                        if sock.send(Message::Text(m.as_str().into())).await.is_err() { break; }
                    }
                    None => break, // dropped by the server (slow consumer, deleted doc)
                }
            }
            inc = tokio::time::timeout(IDLE_TIMEOUT, sock.recv()) => {
                let msg = match inc {
                    Err(_) => break,
                    Ok(None) | Ok(Some(Err(_))) => break,
                    Ok(Some(Ok(m))) => m,
                };
                let text = match msg {
                    Message::Text(t) => t,
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) => continue,
                    Message::Binary(_) => {
                        send_err(&mut sock, "binary frames are not supported").await;
                        break;
                    }
                };
                let parsed: ClientMsg = match serde_json::from_str(&text) {
                    Ok(p) => p,
                    Err(_) => {
                        violations += 1;
                        send_err(&mut sock, "malformed message").await;
                        if violations > 5 { break; }
                        continue;
                    }
                };
                match parsed {
                    ClientMsg::Hello { .. } => {}
                    ClientMsg::Ping => {
                        let m = encode(&ServerMsg::Pong);
                        let _ = tx.try_send(m);
                    }
                    ClientMsg::Cur { x, y } => {
                        if !cur_bucket.take() || !x.is_finite() || !y.is_finite() { continue; }
                        let m = encode(&ServerMsg::Cur { id: cid.clone(), x, y });
                        lock(&active.inner).broadcast(&m, Some(conn));
                    }
                    ClientMsg::Save => {
                        app.flush(&active).await;
                        let seq = lock(&active.inner).seq;
                        let _ = tx.try_send(encode(&ServerMsg::Saved { seq }));
                    }
                    ClientMsg::Ops { ops } => {
                        if !ops_bucket.take() {
                            violations += 1;
                            let _ = tx.try_send(encode(&ServerMsg::Err { msg: "rate limit exceeded".into() }));
                            if violations > 20 { break; }
                            continue;
                        }
                        if ops.len() > MAX_OPS_PER_MSG {
                            let _ = tx.try_send(encode(&ServerMsg::Err { msg: "too many ops in one message".into() }));
                            continue;
                        }
                        apply_ops(&app, &active, conn, &tx, ops);
                    }
                }
            }
        }
    }
    app.leave(&active, conn).await;
}

fn apply_ops(app: &App, active: &Active, conn: u64, tx: &mpsc::Sender<Arc<String>>, ops: Vec<Element>) {
    let mut accepted: Vec<Element> = vec![];
    let mut correct: Vec<Element> = vec![];
    let mut reject: Option<&str> = None;
    let mut g = lock(&active.inner);
    for e in ops {
        if let Err(m) = e.validate() {
            reject = Some(m);
            continue;
        }
        let sz = serde_json::to_string(&e).map(|s| s.len()).unwrap_or(usize::MAX);
        let is_new = !g.doc.els.contains_key(&e.id);
        if is_new && !e.del {
            if g.doc.live_count() >= app.cfg.max_elements || g.doc.els.len() >= app.cfg.max_elements * 4 {
                reject = Some("element limit reached");
                continue;
            }
        }
        let old_sz = g.sizes.get(&e.id).copied().unwrap_or(0);
        if !e.del && g.total + sz > app.cfg.max_doc_bytes + old_sz {
            reject = Some("document size limit reached");
            continue;
        }
        if g.doc.apply(&e) {
            let stored_sz = if e.del { 0 } else { sz };
            g.total = g.total + stored_sz - old_sz.min(g.total + stored_sz);
            g.sizes.insert(e.id.clone(), stored_sz);
            accepted.push(g.doc.els[&e.id].clone());
        } else if let Some(cur) = g.doc.els.get(&e.id) {
            // we lost to a newer version: send the winner back so this client converges
            if cur.v != e.v || cur.by != e.by {
                correct.push(cur.clone());
            }
        }
    }
    if !accepted.is_empty() {
        g.seq += 1;
        g.mark_dirty();
        let m = encode(&ServerMsg::Ops { seq: g.seq, ops: accepted });
        g.broadcast(&m, Some(conn));
    }
    let seq = g.seq;
    drop(g);
    if !correct.is_empty() {
        let _ = tx.try_send(encode(&ServerMsg::Ops { seq, ops: correct }));
    }
    if let Some(m) = reject {
        let _ = tx.try_send(encode(&ServerMsg::Err { msg: m.to_string() }));
    }
}
