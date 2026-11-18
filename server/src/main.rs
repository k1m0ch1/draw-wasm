mod api;
mod db;
mod limits;
mod state;
mod statics;
mod ws;

use axum::routing::{get, post};
use axum::Router;
use state::{App, Config};
use std::net::SocketAddr;
use std::sync::Arc;

fn healthcheck(addr: &str) -> i32 {
    use std::io::{Read, Write};
    let port = addr.rsplit(':').next().unwrap_or("8080");
    let Ok(mut s) = std::net::TcpStream::connect(format!("127.0.0.1:{port}")) else { return 1 };
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(3)));
    let _ = s.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n");
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    if buf.starts_with("HTTP/1.0 200") || buf.starts_with("HTTP/1.1 200") {
        0
    } else {
        1
    }
}

fn admin(cfg: &Config, args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("purge-prefix") => {
            let Some(prefix) = args.get(1).filter(|p| !p.is_empty()) else {
                eprintln!("usage: draw-server admin purge-prefix <prefix>");
                return 2;
            };
            match db::open_conn(std::path::Path::new(&cfg.db_path)).and_then(|c| db::purge_prefix(&c, prefix)) {
                Ok((rooms, docs)) => {
                    println!("purged {rooms} rooms, {docs} documents with prefix {prefix:?}");
                    0
                }
                Err(e) => {
                    eprintln!("purge failed: {e}");
                    1
                }
            }
        }
        Some("stats") => match db::open_conn(std::path::Path::new(&cfg.db_path)) {
            Ok(c) => {
                let n = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(-1);
                println!(
                    "rooms={} docs={} images={}",
                    n("SELECT count(*) FROM rooms"),
                    n("SELECT count(*) FROM docs"),
                    n("SELECT count(*) FROM images")
                );
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        Some("list") => match db::open_conn(std::path::Path::new(&cfg.db_path)) {
            Ok(c) => {
                if let Ok(mut st) = c.prepare("SELECT id FROM rooms ORDER BY created") {
                    if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
                        for r in rows.flatten() {
                            println!("{r}");
                        }
                    }
                }
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        _ => {
            eprintln!("usage: draw-server admin (purge-prefix <prefix> | stats | list)");
            2
        }
    }
}

#[tokio::main]
async fn main() {
    let cfg = Config::from_env();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("healthcheck") => std::process::exit(healthcheck(&cfg.addr)),
        Some("admin") => std::process::exit(admin(&cfg, &args[1..])),
        _ => {}
    }

    let db = match db::Db::open(std::path::Path::new(&cfg.db_path)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("cannot open database: {e}");
            std::process::exit(1);
        }
    };
    let mut salt = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut salt);
    let statics = statics::load(&cfg.static_dir);
    let max_body = cfg.max_image_bytes.max(cfg.max_doc_bytes) + 16 * 1024;
    let addr = cfg.addr.clone();
    let app = Arc::new(App {
        cfg,
        db,
        limiter: Default::default(),
        salt,
        docs: Default::default(),
        next_conn: Default::default(),
        build: statics::build_id(&statics),
        statics,
    });
    tokio::spawn(app.clone().flusher());

    let router = Router::new()
        .route("/healthz", get(api::healthz))
        .route("/api/rooms", post(api::claim_room))
        .route("/api/rooms/{room}", get(api::get_room).delete(api::delete_room))
        .route("/api/rooms/{room}/docs", post(api::create_doc))
        .route("/api/rooms/{room}/docs/{nano}", get(api::get_doc).delete(api::delete_doc))
        .route("/api/rooms/{room}/docs/{nano}/images", post(api::upload_image))
        .route("/api/img/{sha}", get(api::get_image))
        .route("/ws/{room}/{nano}", get(ws::ws_handler))
        .route("/assets/{*path}", get(statics::asset))
        .route("/", get(statics::index))
        .route("/favicon.svg", get(statics::favicon))
        .route("/robots.txt", get(statics::robots))
        .route("/{room}", get(statics::room_page))
        .route("/{room}/{nano}", get(statics::doc_page))
        .fallback(api::not_found)
        .layer(axum::extract::DefaultBodyLimit::max(max_body))
        .with_state(app.clone());

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    println!("listening on {addr}");
    let a2 = app.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        println!("shutting down: flushing documents");
        a2.flush_all().await;
        std::process::exit(0);
    });
    if let Err(e) = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await {
        eprintln!("server error: {e}");
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = term.recv() => {},
                _ = tokio::signal::ctrl_c() => {},
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
