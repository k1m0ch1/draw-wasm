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
    Saved { seq: u64 },
    Err { msg: String },
    Deleted,
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
