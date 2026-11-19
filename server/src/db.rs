//! SQLite access. One dedicated writer thread owns the connection; everything goes through it.
//! All SQL uses bound parameters.

use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::mpsc;
use tokio::sync::oneshot;

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

#[derive(Clone)]
pub struct Db {
    tx: mpsc::Sender<Job>,
}

pub fn open_conn(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "secure_delete", "ON")?;
    migrate(&conn)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for suffix in ["", "-wal", "-shm"] {
            let mut p = path.as_os_str().to_owned();
            p.push(suffix);
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        }
    }
    Ok(conn)
}

/// Idempotent migrations (safe to run at every start).
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rooms (
            id TEXT PRIMARY KEY,
            owner_hash TEXT NOT NULL,
            created INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS docs (
            nano TEXT PRIMARY KEY,
            room TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
            title TEXT NOT NULL DEFAULT '',
            snap TEXT NOT NULL DEFAULT '[]',
            seq INTEGER NOT NULL DEFAULT 0,
            created INTEGER NOT NULL,
            updated INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS docs_room ON docs(room);
         CREATE TABLE IF NOT EXISTS images (
            sha TEXT PRIMARY KEY,
            mime TEXT NOT NULL,
            size INTEGER NOT NULL,
            bytes BLOB NOT NULL,
            created INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS doc_images (
            doc TEXT NOT NULL REFERENCES docs(nano) ON DELETE CASCADE,
            sha TEXT NOT NULL REFERENCES images(sha),
            PRIMARY KEY (doc, sha)
         );
         CREATE INDEX IF NOT EXISTS doc_images_sha ON doc_images(sha);
         CREATE TABLE IF NOT EXISTS grants (
            room TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
            hash TEXT NOT NULL,
            created INTEGER NOT NULL,
            PRIMARY KEY (room, hash)
         );",
    )?;
    // room edit password (argon2id hash, NULL = no password); added after the first release
    let has_pw: bool = conn.prepare("SELECT 1 FROM pragma_table_info('rooms') WHERE name='pw_hash'")?.exists([])?;
    if !has_pw {
        conn.execute_batch("ALTER TABLE rooms ADD COLUMN pw_hash TEXT;")?;
    }
    conn.execute_batch("PRAGMA user_version = 2;")
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Db> {
        let mut conn = open_conn(path)?;
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("db-writer".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    job(&mut conn);
                }
            })
            .expect("spawn db thread");
        Ok(Db { tx })
    }

    pub async fn call<T: Send + 'static>(&self, f: impl FnOnce(&mut Connection) -> T + Send + 'static) -> Option<T> {
        let (otx, orx) = oneshot::channel();
        self.tx
            .send(Box::new(move |c| {
                let _ = otx.send(f(c));
            }))
            .ok()?;
        orx.await.ok()
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Delete orphaned images (not referenced by any document).
pub fn gc_images(conn: &Connection) {
    let _ = conn.execute("DELETE FROM images WHERE sha NOT IN (SELECT sha FROM doc_images)", []);
}

/// Admin: remove every room whose id starts with `prefix`, with its documents and images.
pub fn purge_prefix(conn: &Connection, prefix: &str) -> rusqlite::Result<(usize, usize)> {
    let n = prefix.chars().count() as i64;
    let docs = conn.execute(
        "DELETE FROM docs WHERE room IN (SELECT id FROM rooms WHERE substr(id,1,?1)=?2)",
        params![n, prefix],
    )?;
    let rooms = conn.execute("DELETE FROM rooms WHERE substr(id,1,?1)=?2", params![n, prefix])?;
    gc_images(conn);
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    let _ = conn.execute_batch("VACUUM;");
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    Ok((rooms, docs))
}
