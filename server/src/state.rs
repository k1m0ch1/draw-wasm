//! Shared application state and the in-memory active documents.

use crate::db::{self, Db};
use crate::limits::Limiter;
use proto::{Doc, Element, Peer, ServerMsg};
use rand::Rng;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub struct Config {
    pub addr: String,
    pub db_path: String,
    pub static_dir: String,
    pub public_base_url: Option<String>,
    pub max_image_bytes: usize,
    /// GIFs keep their original bytes, so they get their own (larger) cap
    pub max_gif_bytes: usize,
    pub max_editors: usize,
    pub editor_idle_secs: u64,
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
            max_image_bytes: env_num("DRAW_MAX_IMAGE_BYTES", 2 * 1024 * 1024),
            max_gif_bytes: env_num("DRAW_MAX_GIF_BYTES", 3 * 1024 * 1024),
            max_editors: env_num("DRAW_MAX_EDITORS", 20),
            editor_idle_secs: env_num("DRAW_EDITOR_IDLE_SECS", 600),
            max_doc_image_bytes: env_num("DRAW_MAX_DOC_IMAGE_BYTES", 24 * 1024 * 1024),
            max_doc_images: env_num("DRAW_MAX_DOC_IMAGES", 40),
            max_elements: env_num("DRAW_MAX_ELEMENTS", 5000),
            max_doc_bytes: env_num("DRAW_MAX_DOC_BYTES", 4 * 1024 * 1024),
            max_ws_msg: env_num("DRAW_MAX_WS_MSG", 2 * 1024 * 1024),
            max_conns_per_ip: env_num("DRAW_MAX_CONNS_PER_IP", 30),
            max_docs_per_room: env_num("DRAW_MAX_DOCS_PER_ROOM", 100),
            max_rooms: env_num("DRAW_MAX_ROOMS", 5000),
        }
    }
}

/// What a caller may do in a room. Always computed on the server from the stored state.
#[derive(Clone, Copy, Debug)]
pub struct Rights {
    pub owner: bool,
    pub pw: bool,
    pub edit: bool,
}

/// An editor that left keeps priority for this long; its slot is held for HOLD.
const GRACE: Duration = Duration::from_secs(60);
const HOLD: Duration = Duration::from_secs(4);

pub struct Client {
    pub peer: Peer,
    pub tx: mpsc::Sender<Arc<String>>,
    /// the client's own random id (used only to recognise a reconnecting editor)
    pub cid: String,
    /// rights from the stored room state (password rules); a slot is needed on top of these
    pub rights: Rights,
    /// holds an editing slot (the owner always does and does not count against the limit)
    pub editor: bool,
    /// reconnected within the grace period: ahead of the queue
    pub prio: bool,
    /// join order, the FIFO key of the waitlist
    pub order: u64,
    pub last_act: Instant,
    /// what this client was last told: (edit, pw, queue position, editors)
    pub sent: Option<(bool, bool, u32, u32)>,
}

impl Client {
    pub fn new(peer: Peer, tx: mpsc::Sender<Arc<String>>, cid: String, rights: Rights) -> Client {
        Client { peer, tx, cid, rights, editor: false, prio: false, order: 0, last_act: Instant::now(), sent: None }
    }
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
    /// editors that disconnected recently: (cid, when)
    pub tickets: Vec<(String, Instant)>,
    pub order_ctr: u64,
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
    /// bumped whenever a room's password changes; open sockets re-evaluate their rights when it moves
    pub room_gen: Mutex<HashMap<String, u64>>,
    /// bounds concurrent password hashing (argon2 is memory hungry on purpose)
    pub pw_sem: tokio::sync::Semaphore,
    pub statics: crate::statics::Statics,
    /// asset version of the deployed web bundle (the client reports its own in the hello)
    pub build: String,
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
    /// Hand out editing slots (at most `cap` non-owner editors) in FIFO order of joining, returning
    /// editors first; then tell every client whose state changed. This is the server-side truth for
    /// who may write.
    pub fn rebalance(&mut self, cap: usize, idle: Duration) {
        let now = Instant::now();
        self.tickets.retain(|(_, t)| now.duration_since(*t) < GRACE);
        for c in self.clients.values_mut() {
            if c.peer.owner {
                c.editor = true;
            } else if !c.rights.edit {
                c.editor = false;
            }
        }
        // idle editors give way when somebody is waiting
        if self.clients.values().any(|c| !c.editor && c.rights.edit) {
            let kicked: Vec<u64> = self
                .clients
                .iter()
                .filter(|(_, c)| c.editor && !c.peer.owner && now.duration_since(c.last_act) > idle)
                .map(|(id, _)| *id)
                .collect();
            for id in kicked {
                self.order_ctr += 1;
                let o = self.order_ctr;
                if let Some(c) = self.clients.get_mut(&id) {
                    c.editor = false;
                    c.prio = false;
                    c.order = o;
                    c.last_act = now;
                }
            }
        }
        let mut editors = self.clients.values().filter(|c| c.editor && !c.peer.owner).count();
        let connected: std::collections::HashSet<String> = self.clients.values().map(|c| c.cid.clone()).collect();
        let held = self.tickets.iter().filter(|(cid, t)| now.duration_since(*t) < HOLD && !connected.contains(cid)).count();
        loop {
            let free = cap.saturating_sub(editors);
            if free == 0 {
                break;
            }
            let first = self
                .clients
                .iter()
                .filter(|(_, c)| !c.editor && c.rights.edit)
                .min_by_key(|(_, c)| (!c.prio, c.order))
                .map(|(id, c)| (*id, c.prio));
            let Some((id, prio)) = first else { break };
            if !prio && free <= held {
                break; // that slot is kept for an editor who just dropped out
            }
            if let Some(c) = self.clients.get_mut(&id) {
                c.editor = true;
                c.prio = false;
                c.last_act = now;
            }
            editors += 1;
        }
        let mut q: Vec<(bool, u64, u64)> =
            self.clients.iter().filter(|(_, c)| !c.editor && c.rights.edit).map(|(id, c)| (!c.prio, c.order, *id)).collect();
        q.sort();
        let pos: HashMap<u64, u32> = q.iter().enumerate().map(|(i, (_, _, id))| (*id, i as u32 + 1)).collect();
        for (id, c) in self.clients.iter_mut() {
            let p = pos.get(id).copied().unwrap_or(0);
            let st = (c.editor && c.rights.edit, c.rights.pw, p, editors as u32);
            if c.sent == Some(st) {
                continue;
            }
            if c.sent.map_or(true, |o| o.0 != st.0 || o.1 != st.1) {
                let _ = c.tx.try_send(encode(&ServerMsg::Perm { edit: st.0, pw: st.1 }));
            }
            let _ = c.tx.try_send(encode(&ServerMsg::Role {
                editor: st.0,
                owner: c.peer.owner,
                queued: p > 0,
                pos: p,
                editors: st.3,
                cap: cap as u32,
            }));
            c.sent = Some(st);
        }
    }
    pub fn is_editor(&self, conn: u64) -> bool {
        self.clients.get(&conn).map_or(false, |c| c.editor && c.rights.edit)
    }
    pub fn touch(&mut self, conn: u64) {
        if let Some(c) = self.clients.get_mut(&conn) {
            c.last_act = Instant::now();
        }
    }
    pub fn set_rights(&mut self, conn: u64, r: Rights) {
        if let Some(c) = self.clients.get_mut(&conn) {
            c.rights = r;
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
    pub fn gen(&self, room: &str) -> u64 {
        lock(&self.room_gen).get(room).copied().unwrap_or(0)
    }
    pub fn bump_gen(&self, room: &str) {
        let mut m = lock(&self.room_gen);
        *m.entry(room.to_string()).or_insert(0) += 1;
    }
    /// Rights for a caller holding (optionally) an owner token hash and an edit grant hash.
    /// None when the room does not exist. No password: everybody may edit. Password: only the owner
    /// and holders of a grant issued for THIS room.
    pub async fn rights(&self, room: &str, owner_hash: Option<String>, grant_hash: Option<String>) -> Option<Rights> {
        let r = room.to_string();
        self.db
            .call(move |c| {
                let (oh, pw): (String, Option<String>) = c
                    .query_row("SELECT owner_hash, pw_hash FROM rooms WHERE id=?1", rusqlite::params![r], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })
                    .ok()?;
                let owner = owner_hash.map_or(false, |h| crate::limits::ct_eq(&oh, &h));
                let has_pw = pw.is_some();
                let granted = !owner
                    && has_pw
                    && match grant_hash {
                        Some(g) => c
                            .query_row("SELECT 1 FROM grants WHERE room=?1 AND hash=?2", rusqlite::params![r, g], |_| Ok(()))
                            .is_ok(),
                        None => false,
                    };
                Some(Rights { owner, pw: has_pw, edit: owner || !has_pw || granted })
            })
            .await
            .flatten()
    }

    /// Load (or fetch from memory) the active document and register a client. Returns None when the
    /// document does not exist or does not belong to `room`.
    pub async fn join(
        &self,
        room: &str,
        nano: &str,
        conn: u64,
        client: Client,
    ) -> Option<(Arc<Active>, ServerMsg)> {
        let mut client = client;
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
                        tickets: vec![],
                        order_ctr: 0,
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
            if client.peer.name.is_empty() {
                let used: std::collections::HashSet<String> = g.clients.values().map(|c| c.peer.name.clone()).collect();
                client.peer.name = random_face(&used);
            }
            let you = client.peer.clone();
            let peers: Vec<Peer> = g.clients.values().map(|c| c.peer.clone()).collect();
            let m = encode(&ServerMsg::Peer { peer: you.clone() });
            g.broadcast(&m, None);
            g.order_ctr += 1;
            client.order = g.order_ctr;
            if let Some(i) = g.tickets.iter().position(|(c, _)| *c == client.cid) {
                g.tickets.remove(i);
                client.prio = true;
            }
            g.clients.insert(conn, client);
            g.rebalance(self.cfg.max_editors, Duration::from_secs(self.cfg.editor_idle_secs));
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
            let removed = g.clients.remove(&conn);
            if let Some(c) = &removed {
                if c.editor && !c.peer.owner && !c.cid.is_empty() {
                    let cid = c.cid.clone();
                    g.tickets.retain(|(x, _)| *x != cid);
                    g.tickets.push((cid, Instant::now()));
                }
            }
            g.rebalance(self.cfg.max_editors, Duration::from_secs(self.cfg.editor_idle_secs));
            let gone = removed.map(|c| c.peer.id);
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
                    let mut g = lock(&a.inner);
                    g.rebalance(self.cfg.max_editors, Duration::from_secs(self.cfg.editor_idle_secs));
                    g.dirty && (g.last_edit.elapsed().as_secs() >= 2 || g.first_dirty.elapsed().as_secs() >= 8)
                };
                if due {
                    self.flush(&a).await;
                }
            }
        }
    }

    /// Delete a document only when it belongs to `room` (the DB delete is also scoped by room).
    pub async fn drop_doc_in(&self, room: &str, nano: &str) {
        let belongs = match self.docs.lock().await.get(nano) {
            Some(a) => a.room == room,
            None => true,
        };
        if belongs {
            self.drop_doc(nano).await;
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

pub enum NameCheck {
    Empty,
    Ok(String),
    Bad,
}

/// Display name rules: trimmed, at most 32 characters, no control characters, only letters, digits,
/// spaces and a few punctuation marks, OR exactly one face emoji. Empty is allowed (a face is assigned).
pub fn name_check(s: &str) -> NameCheck {
    let t = s.trim();
    let n = t.chars().count();
    if n == 0 {
        return NameCheck::Empty;
    }
    if n > 32 {
        return NameCheck::Bad;
    }
    if n == 1 && t.chars().next().map_or(false, proto::is_face) {
        return NameCheck::Ok(t.to_string());
    }
    if !t.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '\'')) {
        return NameCheck::Bad;
    }
    NameCheck::Ok(t.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// A random face emoji, avoiding the ones `used` already when possible.
pub fn random_face(used: &std::collections::HashSet<String>) -> String {
    let all = proto::faces();
    let free: Vec<char> = all.iter().copied().filter(|c| !used.contains(&c.to_string())).collect();
    let pool = if free.is_empty() { &all } else { &free };
    pool[rand::thread_rng().gen_range(0..pool.len())].to_string()
}

pub fn random_id(n: usize) -> String {
    const A: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut r = rand::thread_rng();
    (0..n).map(|_| A[r.gen_range(0..A.len())] as char).collect()
}

pub fn nanoid() -> String {
    let mut r = rand::thread_rng();
    (0..proto::NANO_LEN)
        .map(|_| proto::NANO_ALPHABET[r.gen_range(0..64)] as char)
        .collect()
}
