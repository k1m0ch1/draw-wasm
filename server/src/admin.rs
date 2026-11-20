// Read-only admin overview: live rooms, participants, recent drawing activity.
// Gated by DRAW_ADMIN_KEY (the routes 404 when it is empty); the key is injected by the
// reverse proxy of the admin host, never by a browser. Responses carry no ids, tokens or IPs.

use crate::state::App;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::time::Instant;

/// How many recent events are kept in memory (and returned, newest first).
pub const RECENT_MAX: usize = 200;

pub struct RecentEvent {
    pub at: Instant,
    pub room: String,
    pub nano: String,
    pub who: String,
    pub what: String,
}

impl App {
    /// Called on every accepted batch of ops (bounded, cheap).
    pub fn record_recent(&self, room: &str, nano: &str, who: &str, what: &str) {
        let mut q = crate::state::lock(&self.recent);
        q.push_back(RecentEvent {
            at: Instant::now(),
            room: room.to_string(),
            nano: nano.to_string(),
            who: who.to_string(),
            what: what.to_string(),
        });
        while q.len() > RECENT_MAX {
            q.pop_front();
        }
    }
}

/// One short human-readable line per element op, e.g. `rect "todo list"` or `deleted text`.
pub fn describe(e: &proto::Element) -> String {
    let snippet: String = e
        .tx
        .chars()
        .take(32)
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string();
    let kind = match e.k {
        proto::Kind::Rect => "rect",
        proto::Kind::Ellipse => "circle",
        proto::Kind::Diamond => "diamond",
        proto::Kind::Arrow => "arrow",
        proto::Kind::Pencil => "pencil",
        proto::Kind::Text => "text",
        proto::Kind::Image => "image",
        proto::Kind::Html => "html object",
    };
    let base = if snippet.is_empty() { kind.to_string() } else { format!("{kind} \"{snippet}\"") };
    if e.del { format!("deleted {base}") } else { base }
}

/// The whole admin payload. Cheap: DB is asked for three counts, live docs are snapshotted
/// under their short-lived locks, recent is copied out.
pub async fn overview(app: &App) -> Value {
    let counts = app
        .db
        .call(|c| {
            let n = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(-1);
            (n("SELECT count(*) FROM rooms"), n("SELECT count(*) FROM docs"), n("SELECT count(*) FROM images"))
        })
        .await
        .unwrap_or((-1, -1, -1));

    let mut docs_json: Vec<Value> = vec![];
    let (mut conns, mut editors, mut viewers) = (0u64, 0u64, 0u64);
    {
        let docs = app.docs.lock().await;
        for a in docs.values() {
            let g = crate::state::lock(&a.inner);
            let mut people = vec![];
            for c in g.clients.values() {
                let role = if c.peer.owner { "owner" } else if c.editor { "editor" } else { "viewer" };
                let name = if c.peer.name.is_empty() { "anon".to_string() } else { c.peer.name.clone() };
                people.push(json!({ "name": name, "role": role }));
            }
            conns += people.len() as u64;
            editors += people.iter().filter(|p| p["role"] == "editor" || p["role"] == "owner").count() as u64;
            viewers += people.iter().filter(|p| p["role"] == "viewer").count() as u64;
            docs_json.push(json!({
                "room": a.room,
                "nano": a.nano,
                "title": g.title,
                "participants": people,
                "editors": people.iter().filter(|p| p["role"] != "viewer").count(),
                "viewers": people.iter().filter(|p| p["role"] == "viewer").count(),
                "elements": g.doc.live_count(),
                "last_op_secs": g.last_op.elapsed().as_secs(),
            }));
        }
        docs_json.sort_by(|a, b| b["last_op_secs"].as_u64().cmp(&a["last_op_secs"].as_u64()));
    }

    let recent: Vec<Value> = {
        let q = crate::state::lock(&app.recent);
        q.iter()
            .rev()
            .map(|e| {
                json!({
                    "ago_secs": e.at.elapsed().as_secs(),
                    "room": e.room,
                    "nano": e.nano,
                    "who": e.who,
                    "what": e.what,
                })
            })
            .collect()
    };

    json!({
        "build": app.build,
        "db": { "rooms": counts.0, "docs": counts.1, "images": counts.2 },
        "live": { "docs": docs_json.len(), "connections": conns, "editors": editors, "viewers": viewers },
        "docs": docs_json,
        "recent": recent,
    })
}


/// Rooms in the database: name, age, password flag, doc count, latest doc update.
pub async fn rooms_list(app: &App) -> Value {
    let rooms = app
        .db
        .call(|c| {
            let mut st = c
                .prepare("SELECT r.id, r.created, r.pw_hash IS NOT NULL, count(d.nano), coalesce(max(d.updated), r.created) FROM rooms r LEFT JOIN docs d ON d.room = r.id GROUP BY r.id ORDER BY 5 DESC LIMIT 500")
                .expect("rooms q");
            let rows = st
                .query_map([], |r| {
                    Ok(json!({ "room": r.get::<_, String>(0)?, "created": r.get::<_, i64>(1)?, "pw": r.get::<_, i64>(2)? != 0, "docs": r.get::<_, i64>(3)?, "last": r.get::<_, i64>(4)? }))
                })
                .expect("rooms map")
                .filter_map(|x| x.ok())
                .collect::<Vec<_>>();
            rows
        })
        .await
        .unwrap_or_default();
    json!({ "rooms": rooms })
}

/// Documents in the database, newest first, with a live element count from the snapshot.
pub async fn docs_list(app: &App, limit: i64, offset: i64) -> Value {
    let (total, docs) = app
        .db
        .call(move |c| {
            let total: i64 = c.query_row("SELECT count(*) FROM docs", [], |r| r.get(0)).unwrap_or(0);
            let mut st = c
                .prepare("SELECT nano, room, title, updated, created, snap FROM docs ORDER BY updated DESC LIMIT ?1 OFFSET ?2")
                .expect("docs q");
            let rows = st
                .query_map([limit, offset], |r| {
                    let snap: String = r.get(5)?;
                    let elements = serde_json::from_str::<Vec<Value>>(&snap).map(|v| v.len()).unwrap_or(0);
                    Ok(json!({ "nano": r.get::<_, String>(0)?, "room": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?, "updated": r.get::<_, i64>(3)?, "created": r.get::<_, i64>(4)?, "elements": elements }))
                })
                .expect("docs map")
                .filter_map(|x| x.ok())
                .collect::<Vec<_>>();
            (total, rows)
        })
        .await
        .unwrap_or((0, vec![]));
    json!({ "total": total, "docs": docs })
}

/// Images in the database (metadata only; the bytes are fetched per-thumb from the public image route).
pub async fn images_list(app: &App, limit: i64, offset: i64) -> Value {
    let (total, images) = app
        .db
        .call(move |c| {
            let total: i64 = c.query_row("SELECT count(*) FROM images", [], |r| r.get(0)).unwrap_or(0);
            let mut st = c
                .prepare("SELECT i.sha, i.mime, i.size, i.created, (SELECT count(*) FROM doc_images di WHERE di.sha = i.sha) FROM images i ORDER BY i.created DESC LIMIT ?1 OFFSET ?2")
                .expect("images q");
            let rows = st
                .query_map([limit, offset], |r| {
                    Ok(json!({ "sha": r.get::<_, String>(0)?, "mime": r.get::<_, String>(1)?, "size": r.get::<_, i64>(2)?, "created": r.get::<_, i64>(3)?, "used_by": r.get::<_, i64>(4)? }))
                })
                .expect("images map")
                .filter_map(|x| x.ok())
                .collect::<Vec<_>>();
            (total, rows)
        })
        .await
        .unwrap_or((0, vec![]));
    json!({ "total": total, "images": images })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_names_kinds_and_snippets() {
        let mut e = proto::Element::default();
        e.k = proto::Kind::Rect;
        e.tx = "kumpulin fakta".into();
        assert_eq!(describe(&e), "rect \"kumpulin fakta\"");
        e.del = true;
        assert_eq!(describe(&e), "deleted rect \"kumpulin fakta\"");
        e.tx = String::new();
        e.del = false;
        e.k = proto::Kind::Html;
        assert_eq!(describe(&e), "html object");
    }

    #[test]
    fn recent_buffer_is_bounded() {
        let app = crate::state::tests::app_for_tests();
        for i in 0..(RECENT_MAX + 40) {
            app.record_recent("r", "n", "who", &format!("op {i}"));
        }
        let q = crate::state::lock(&app.recent);
        assert_eq!(q.len(), RECENT_MAX);
        assert_eq!(q.back().map(|e| e.what.as_str()), Some(format!("op {}", RECENT_MAX + 39).as_str()));
    }

    #[tokio::test]
    async fn overview_has_shape_and_no_secrets() {
        let app = crate::state::tests::app_for_tests();
        app.record_recent("room1", "nano1", "saya", "rect \"hello\"");
        let v = overview(&app).await;
        assert_eq!(v["live"]["docs"].as_u64(), Some(0));
        assert_eq!(v["recent"][0]["who"].as_str(), Some("saya"));
        let s = v.to_string();
        assert!(!s.contains("token") && !s.contains("cid") && !s.contains("grant"));
    }
}
