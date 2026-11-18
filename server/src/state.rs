//! Shared application state and the in-memory active documents.

use crate::db::{self, Db};
use crate::limits::Limiter;
use proto::{Doc, Element, Peer, ServerMsg};
use rand::Rng;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;

pub struct Config {
    pub addr: String,
    pub db_path: String,
    pub static_dir: String,
    pub public_base_url: Option<String>,
    pub max_image_bytes: usize,
    pub max_doc_image_bytes: i64,
    pub max_doc_images: i64,
    pub max_elements: usize,
    pub max_doc_bytes: usize,
    pub max_ws_msg: usize,
    pub max_conns_per_ip: usize,
    pub max_docs_per_room: i64,
    pub max_rooms: i64,
}

fn env_num<T: std::str::FromStr>(k: &str, d: T) -> T {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

impl Config {
    pub fn from_env() -> Config {
        Config {
            addr: std::env::var("DRAW_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            db_path: std::env::var("DRAW_DB").unwrap_or_else(|_| "/data/draw.db".into()),
            static_dir: std::env::var("DRAW_STATIC").unwrap_or_else(|_| "/app/static".into()),
            public_base_url: std::env::var("PUBLIC_BASE_URL").ok().filter(|s| !s.is_empty()),
            max_image_bytes: env_num("DRAW_MAX_IMAGE_BYTES", 4 * 1024 * 1024),
            max_doc_image_bytes: env_num("DRAW_MAX_DOC_IMAGE_BYTES", 24 * 1024 * 1024),
            max_doc_images: env_num("DRAW_MAX_DOC_IMAGES", 40),
            max_elements: env_num("DRAW_MAX_ELEMENTS", 5000),
            max_doc_bytes: env_num("DRAW_MAX_DOC_BYTES", 4 * 1024 * 1024),
            max_ws_msg: env_num("DRAW_MAX_WS_MSG", 1024 * 1024),
            max_conns_per_ip: env_num("DRAW_MAX_CONNS_PER_IP", 20),
            max_docs_per_room: env_num("DRAW_MAX_DOCS_PER_ROOM", 100),
            max_rooms: env_num("DRAW_MAX_ROOMS", 5000),
        }
    }
}

pub struct Client {
    pub peer: Peer,
    pub tx: mpsc::Sender<Arc<String>>,
}

pub struct Inner {
    pub doc: Doc,
    pub seq: u64,
    pub title: String,
    pub dirty: bool,
    pub last_edit: Instant,
    pub first_dirty: Instant,
    pub sizes: HashMap<String, usize>,
    pub total: usize,
    pub clients: HashMap<u64, Client>,
    pub deleted: bool,
}

pub struct Active {
    pub nano: String,
    pub room: String,
    pub inner: Mutex<Inner>,
}

pub struct App {
    pub cfg: Config,
    pub db: Db,
    pub limiter: Limiter,
    pub salt: [u8; 16],
    pub docs: tokio::sync::Mutex<HashMap<String, Arc<Active>>>,
    pub next_conn: std::sync::atomic::AtomicU64,
    pub statics: crate::statics::Statics,
}

pub fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn encode(m: &ServerMsg) -> Arc<String> {
    Arc::new(serde_json::to_string(m).unwrap_or_else(|_| "{\"t\":\"err\",\"msg\":\"internal\"}".into()))
}

impl Inner {
    /// Send to everyone except `skip`; clients whose queue is full are dropped (they resync on reconnect).
    pub fn broadcast(&mut self, msg: &Arc<String>, skip: Option<u64>) {
        let mut dead = vec![];
        for (id, c) in &self.clients {
            if Some(*id) == skip {
                continue;
            }
            if c.tx.try_send(msg.clone()).is_err() {
                dead.push(*id);
            }
        }
        for id in dead {
            self.clients.remove(&id);
        }
    }
    pub fn snapshot_json(&self) -> String {
        let v: Vec<&Element> = self.doc.els.values().collect();
        serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
    }
    pub fn mark_dirty(&mut self) {
        let now = Instant::now();
        if !self.dirty {
            self.first_dirty = now;
        }
        self.dirty = true;
        self.last_edit = now;
    }
}

impl App {
    /// Load (or fetch from memory) the active document and register a client. Returns None when the
    /// document does not exist or does not belong to `room`.
    pub async fn join(
        &self,
        room: &str,
        nano: &str,
        conn: u64,
        client: Client,
    ) -> Option<(Arc<Active>, ServerMsg)> {
        let mut map = self.docs.lock().await;
        let active = match map.get(nano) {
            Some(a) if a.room == room => a.clone(),
            Some(_) => return None,
            None => {
                let (r, n) = (room.to_string(), nano.to_string());
                let row: Option<(String, String, i64)> = self
                    .db
                    .call(move |c| {
                        c.query_row(
                            "SELECT title, snap, seq FROM docs WHERE nano=?1 AND room=?2",
                            rusqlite::params![n, r],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                        )
                        .ok()
                    })
                    .await?;
                let (title, snap, seq) = row?;
                let els: Vec<Element> = serde_json::from_str(&snap).unwrap_or_default();
                let mut doc = Doc::default();
                let mut sizes = HashMap::new();
                let mut total = 0;
                for e in els {
                    if e.validate().is_ok() {
                        let sz = serde_json::to_string(&e).map(|s| s.len()).unwrap_or(0);
                        total += sz;
                        sizes.insert(e.id.clone(), sz);
                        doc.apply(&e);
                    }
                }
                let now = Instant::now();
                let a = Arc::new(Active {
                    nano: nano.to_string(),
                    room: room.to_string(),
                    inner: Mutex::new(Inner {
                        doc,
                        seq: seq.max(0) as u64,
                        title,
                        dirty: false,
                        last_edit: now,
                        first_dirty: now,
                        sizes,
                        total,
                        clients: HashMap::new(),
                        deleted: false,
                    }),
                });
                map.insert(nano.to_string(), a.clone());
                a
            }
        };
        let snap = {
            let mut g = lock(&active.inner);
            if g.deleted {
                return None;
            }
            let you = client.peer.clone();
            let peers: Vec<Peer> = g.clients.values().map(|c| c.peer.clone()).collect();
            let m = encode(&ServerMsg::Peer { peer: you.clone() });
            g.broadcast(&m, None);
            g.clients.insert(conn, client);
            ServerMsg::Snap {
                seq: g.seq,
                title: g.title.clone(),
                els: g.doc.els.values().cloned().collect(),
                you,
                peers,
            }
        };
        Some((active, snap))
    }

    /// Remove a client; when it was the last one the document is flushed and dropped from memory.
    pub async fn leave(&self, a: &Arc<Active>, conn: u64) {
        let mut map = self.docs.lock().await;
        let (empty, gone) = {
            let mut g = lock(&a.inner);
            let gone = g.clients.remove(&conn).map(|c| c.peer.id);
            if let Some(id) = &gone {
                let m = encode(&ServerMsg::Gone { id: id.clone() });
                g.broadcast(&m, None);
            }
            (g.clients.is_empty(), gone)
        };
        let _ = gone;
        if empty {
            self.flush(a).await;
            map.remove(&a.nano);
        }
    }

    pub async fn flush(&self, a: &Arc<Active>) {
        let (snap, seq) = {
            let mut g = lock(&a.inner);
            if !g.dirty || g.deleted {
                return;
            }
            g.dirty = false;
            // keep the stored document bounded: drop tombstones when there are far too many
            let tombs = g.doc.els.values().filter(|e| e.del).count();
            if tombs > 20_000 {
                g.doc.els.retain(|_, e| !e.del);
            }
            (g.snapshot_json(), g.seq)
        };
        let nano = a.nano.clone();
        let ok = self
            .db
            .call(move |c| {
                c.execute(
                    "UPDATE docs SET snap=?1, seq=?2, updated=?3 WHERE nano=?4",
                    rusqlite::params![snap, seq as i64, db::now(), nano],
                )
                .is_ok()
            })
            .await
            .unwrap_or(false);
        if !ok {
            lock(&a.inner).dirty = true;
            eprintln!("flush failed for a document; will retry");
        }
    }

    pub async fn flush_all(&self) {
        let list: Vec<Arc<Active>> = self.docs.lock().await.values().cloned().collect();
        for a in list {
            self.flush(&a).await;
        }
    }

    pub async fn flusher(self: Arc<Self>) {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tick.tick().await;
            let list: Vec<Arc<Active>> = self.docs.lock().await.values().cloned().collect();
            for a in list {
                let due = {
                    let g = lock(&a.inner);
                    g.dirty && (g.last_edit.elapsed().as_secs() >= 2 || g.first_dirty.elapsed().as_secs() >= 8)
                };
                if due {
                    self.flush(&a).await;
                }
            }
        }
    }

    /// Owner deleted a document: tell the clients and forget it.
    pub async fn drop_doc(&self, nano: &str) {
        let mut map = self.docs.lock().await;
        if let Some(a) = map.remove(nano) {
            let mut g = lock(&a.inner);
            g.deleted = true;
            let m = encode(&ServerMsg::Deleted);
            g.broadcast(&m, None);
            g.clients.clear();
        }
    }
}

const COLORS: &[&str] = &[
    "#e8590c", "#2f9e44", "#1c7ed6", "#ae3ec9", "#e03131", "#0c8599", "#f08c00", "#5c7cfa", "#d6336c", "#37b24d",
];

pub fn random_color() -> String {
    COLORS[rand::thread_rng().gen_range(0..COLORS.len())].to_string()
}

/// Display name rules: trimmed, 1..=32 characters, no control characters, only letters, digits,
/// spaces and a few punctuation marks. Returns None when invalid.
pub fn clean_name(s: &str) -> Option<String> {
    let t = s.trim();
    let n = t.chars().count();
    if n == 0 || n > 32 {
        return None;
    }
    if !t.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | ''')) {
        return None;
    }
    Some(t.split_whitespace().collect::<Vec<_>>().join(" "))
}

pub fn nanoid() -> String {
    let mut r = rand::thread_rng();
    (0..proto::NANO_LEN)
        .map(|_| proto::NANO_ALPHABET[r.gen_range(0..64)] as char)
        .collect()
}
