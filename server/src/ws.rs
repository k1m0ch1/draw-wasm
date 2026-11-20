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
use std::time::{Duration, Instant};
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

/// Hash of a presented secret (owner token or edit grant) when it is well formed.
fn secret_hash(t: &str) -> Option<String> {
    if t.len() < 20 || t.len() > 128 || !t.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    Some(crate::limits::sha256_hex(t.as_bytes()))
}

async fn handle(app: Arc<App>, mut sock: WebSocket, room: String, nano: String, key: String) {
    // 1. hello (must arrive quickly)
    let hello = tokio::time::timeout(Duration::from_secs(10), sock.recv()).await;
    let (name, token, build, grant, cid) = match hello {
        Ok(Some(Ok(Message::Text(t)))) => match serde_json::from_str::<ClientMsg>(&t) {
            Ok(ClientMsg::Hello { cid, name, token, build, grant, .. }) if proto::valid_id(&cid) => (name, token, build, grant, cid),
            _ => {
                send_err(&mut sock, "expected hello").await;
                return;
            }
        },
        _ => return,
    };
    // Rights are decided here from the stored state only; nothing the client says about itself counts.
    let (oh, gh) = (secret_hash(&token), secret_hash(&grant));
    let Some(rights) = app.rights(&room, oh.clone(), gh.clone()).await else {
        send_err(&mut sock, "document not found").await;
        return;
    };
    let seen_gen0 = app.gen(&room);
    // empty name = the server picks a random face emoji when the client joins the document
    let name = match name_check(&name) {
        NameCheck::Ok(n) => n,
        NameCheck::Empty => String::new(),
        NameCheck::Bad => {
            send_err(&mut sock, "invalid display name (at most 32 letters, digits, spaces or - _ . ')").await;
            return;
        }
    };
    let conn = app.next_conn.fetch_add(1, Ordering::Relaxed);
    let (tx, mut rx) = mpsc::channel::<Arc<String>>(512);
    // the participant id other clients see is chosen by the server: it cannot be spoofed or collide
    let pid = format!("p{}", random_id(11));
    let peer = Peer { id: pid.clone(), name, color: random_color(), owner: rights.owner };
    let Some((active, snap)) = app.join(&room, &nano, conn, Client::new(peer, tx.clone(), cid, rights)).await else {
        send_err(&mut sock, "document not found").await;
        return;
    };
    // tell a client on another build first, so it can keep its unsent edits; it still gets the snapshot
    if build != app.build && !app.build.is_empty() {
        let _ = sock.send(Message::Text(encode(&ServerMsg::Reload { build: app.build.clone() }).as_str().into())).await;
    }

    // Perm and Role (editor / viewer + queue position) were queued by the join and follow the snapshot
    let first = encode(&snap);
    if sock.send(Message::Text(first.as_str().into())).await.is_err() {
        app.leave(&active, conn).await;
        return;
    }

    let mut seen_gen = seen_gen0;
    let mut ops_bucket = LocalBucket::new(60.0, 30.0);
    let mut cur_bucket = LocalBucket::new(40.0, 30.0);
    let mut violations = 0u32;
    let _ = &key;

    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            _ = tick.tick() => {
                refresh_rights(&app, &active, conn, &room, &oh, &gh, &mut seen_gen).await;
            }
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
                        let m = encode(&ServerMsg::Cur { id: pid.clone(), x, y });
                        let mut g = lock(&active.inner);
                        g.touch(conn);
                        g.broadcast(&m, Some(conn));
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
                        refresh_rights(&app, &active, conn, &room, &oh, &gh, &mut seen_gen).await;
                        let allowed = {
                            let mut g = lock(&active.inner);
                            let a = g.is_editor(conn);
                            if a { g.touch(conn); }
                            a
                        };
                        if !allowed {
                            reject_ops(&active, &tx, ops, rights.edit);
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
    let who = {
        let g = lock(&active.inner);
        g.clients.get(&conn).map(|c| if c.peer.name.is_empty() { "(anon)".to_string() } else { c.peer.name.clone() }).unwrap_or_default()
    };
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
        g.last_op = Instant::now();
        g.mark_dirty();
        let names: Vec<String> = accepted.iter().map(|e| crate::admin::describe(e)).collect();
        app.record_recent(&active.room, &active.nano, &who, &names.join(", "));
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

/// A read-only connection tried to write. Nothing is accepted; the writer's copy is put back to the
/// server's state with a newer version so the client converges (and everybody else is unaffected
/// content-wise). The client is told why.
fn reject_ops(active: &Active, tx: &mpsc::Sender<Arc<String>>, ops: Vec<Element>, may_edit: bool) {
    let mut g = lock(&active.inner);
    let mut fix: Vec<Element> = vec![]; // known elements: the stored state, re-versioned (goes to everyone)
    let mut back: Vec<Element> = vec![]; // unknown elements: a tombstone, for the sender only
    for e in ops.iter().take(MAX_OPS_PER_MSG) {
        if e.validate().is_err() || e.v >= u32::MAX - 2 {
            continue;
        }
        match g.doc.els.get(&e.id).cloned() {
            Some(mut r) => {
                r.v = r.v.max(e.v) + 1;
                r.by = "srv".to_string();
                if g.doc.apply(&r) {
                    fix.push(r);
                }
            }
            None => {
                let mut t = e.clone();
                t.v = e.v + 1;
                t.by = "srv".to_string();
                t.del = true;
                t.pts.clear();
                t.tx.clear();
                t.src.clear();
                back.push(t);
            }
        }
    }
    if !fix.is_empty() {
        g.seq += 1;
        g.mark_dirty();
        let m = encode(&ServerMsg::Ops { seq: g.seq, ops: fix });
        g.broadcast(&m, None); // includes the writer, whose copy is reset to the server's state
    }
    let seq = g.seq;
    drop(g);
    if !back.is_empty() {
        let _ = tx.try_send(encode(&ServerMsg::Ops { seq, ops: back }));
    }
    let why = if may_edit {
        "you are viewing: all editing slots are taken, you will be promoted when one frees up"
    } else {
        "this room is read-only: enter the edit password to make changes"
    };
    let _ = tx.try_send(encode(&ServerMsg::Err { msg: why.into() }));
}

/// Re-read this connection's rights when the room's password changed (set, changed or removed); the
/// slot logic then tells the client. Runs on a timer and before every write, so a revoked grant stops
/// working at once.
async fn refresh_rights(
    app: &App,
    active: &Active,
    conn: u64,
    room: &str,
    oh: &Option<String>,
    gh: &Option<String>,
    seen: &mut u64,
) {
    let g = app.gen(room);
    if g == *seen {
        return;
    }
    *seen = g;
    if let Some(r) = app.rights(room, oh.clone(), gh.clone()).await {
        let mut a = lock(&active.inner);
        a.set_rights(conn, r);
        a.rebalance(app.cfg.max_editors, Duration::from_secs(app.cfg.editor_idle_secs));
    }
}
