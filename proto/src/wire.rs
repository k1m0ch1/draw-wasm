//! WebSocket protocol (JSON text frames) and identifier rules.

use crate::Element;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub owner: bool,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ClientMsg {
    Hello {
        cid: String,
        #[serde(default)]
        name: String,
        /// room ownership token (optional); the server only compares its hash
        #[serde(default)]
        token: String,
        /// last sequence number seen; the server always answers with a fresh snapshot
        #[serde(default)]
        seq: u64,
        /// build id of the page the client runs (absent in clients older than the handshake)
        #[serde(default)]
        build: String,
        /// edit grant issued by the server after the room password was entered (optional)
        #[serde(default)]
        grant: String,
    },
    Ops { ops: Vec<Element> },
    Cur { x: f64, y: f64 },
    Ping,
    Save,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ServerMsg {
    Snap {
        seq: u64,
        title: String,
        els: Vec<Element>,
        you: Peer,
        peers: Vec<Peer>,
    },
    Ops { seq: u64, ops: Vec<Element> },
    Peer { peer: Peer },
    Gone { id: String },
    Cur { id: String, x: f64, y: f64 },
    Pong,
    /// What this connection may do. Computed by the server only; sent before the snapshot and whenever it changes.
    Perm { edit: bool, pw: bool },
    Saved { seq: u64 },
    Err { msg: String },
    Deleted,
    /// The client runs a different build than the server: it should reload the page.
    Reload { build: String },
}

/// roomID rules: lowercase [a-z0-9-], 3..32 chars, no leading/trailing hyphen, not reserved.
pub fn valid_room(s: &str) -> Result<(), &'static str> {
    if s.len() < 3 || s.len() > 32 {
        return Err("room id must be 3 to 32 characters");
    }
    if !s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return Err("room id may only contain a-z, 0-9 and hyphens");
    }
    if s.starts_with('-') || s.ends_with('-') {
        return Err("room id must not start or end with a hyphen");
    }
    if RESERVED.contains(&s) {
        return Err("this room id is reserved");
    }
    Ok(())
}

pub const RESERVED: &[&str] = &[
    "api", "ws", "assets", "static", "admin", "healthz", "health", "new", "draw", "app", "robots",
    "favicon", "dist", "index", "login", "logout", "signup", "register", "root", "null", "undefined",
    "www", "mail", "docs", "doc", "room", "rooms", "img", "image", "images", "public", "private",
    "settings", "status", "metrics", "sitemap", "manifest", "sw", "support", "help", "about",
    "terms", "privacy", "system", "config", "local", "download", "upload",
];

pub const NANO_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
pub const NANO_LEN: usize = 10;

pub fn valid_nano(s: &str) -> bool {
    s.len() == NANO_LEN && s.bytes().all(|b| NANO_ALPHABET.contains(&b))
}

/// Code point ranges of the emoji that depict a face (never hands, gesturing people or objects).
/// The same set is handed out to joiners without a name and accepted as a typed name.
pub const FACE_RANGES: &[(u32, u32)] = &[
    (0x1F600, 0x1F637),
    (0x1F641, 0x1F644),
    (0x1F910, 0x1F917),
    (0x1F920, 0x1F925),
    (0x1F927, 0x1F92F),
    (0x1F970, 0x1F976),
    (0x1F978, 0x1F97A),
];
pub fn is_face(c: char) -> bool {
    let u = c as u32;
    FACE_RANGES.iter().any(|(a, b)| u >= *a && u <= *b)
}

/// The face emoji handed out to joiners without a name.
pub fn faces() -> Vec<char> {
    FACE_RANGES.iter().flat_map(|(a, b)| (*a..=*b).filter_map(char::from_u32)).collect()
}
