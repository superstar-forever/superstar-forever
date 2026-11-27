//! Offline replacement server for Dalcomsoft's SuperStar games.
//!
//! Routes:
//!   GET  /health                          -> "ok"
//!   GET  /save                            -> the player's save (read-only, cards omitted)
//!   GET  /save/full, PUT /save/full       -> export / replace the whole save (tools/save.py)
//!   GET  .../manifest/<version>.txt       -> startup manifest (points the game at us)
//!   GET  /cdn/<host>/<path>               -> game files (images, songs, charts, master data)
//!   POST anything else                    -> RPC API (see api.rs)

#[cfg(target_os = "android")]
pub mod android;
pub mod api;
pub mod assets;
pub mod crypto;
pub mod data;
pub mod http;
pub mod progression;
pub mod state;

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use http::{Request, Response};

pub struct Config {
    /// server-data/ folder, or the zip(s) holding it (the data split APKs on Android).
    pub data: Vec<PathBuf>,
    /// Address to listen on, e.g. "127.0.0.1:18384" or "0.0.0.0:18384".
    pub bind: String,
    /// Base URL the game should use to reach us, e.g. "http://192.168.1.10:18384".
    pub public_url: String,
    /// Save file for the player.
    pub save: PathBuf,
    /// Optional log file (in addition to stderr).
    pub log: Option<PathBuf>,
}

struct App {
    assets: assets::Assets,
    api: api::Api,
    log: Option<std::sync::Mutex<std::fs::File>>,
}

impl App {
    fn log(&self, line: &str) {
        eprintln!("{line}");
        if let Some(f) = &self.log {
            if let Ok(mut f) = f.lock() {
                let _ = writeln!(f, "{} {line}", state::now_ms());
            }
        }
    }
}

/// Starts the server and blocks forever (or until binding fails).
pub fn run(cfg: Config) -> Result<(), String> {
    let data = data::DataSource::open(&cfg.data)
        .map_err(|e| format!("cannot open data {:?}: {e}", cfg.data))?;
    let key = data.read("game.key").ok_or("game.key missing from the data (tools/build-server-data.py)")?;
    crypto::set_key(&key)?;
    let assets = assets::Assets::new(data, &cfg.public_url);

    let defaults = assets
        .data()
        .read("api-defaults.json")
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let prog = progression::Progression::load(&assets);
    let card_count = prog.card_count();
    let player_default = defaults
        .get("Account.login")
        .map(|l| l["user"].clone())
        .unwrap_or_default();
    let state = state::State::load_or_create(cfg.save.clone(), player_default, &prog);
    let profile_codes: Vec<i64> = assets
        .table("ProfileData")
        .and_then(|t| t.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|p| p["code"].as_i64())
        .collect();

    // A log that can't be opened must not keep the server from starting: on a phone's very first
    // start, Android/data/<package>/files doesn't exist yet when the server inside the app starts
    // (Unity creates it a moment later), so the game showed "Please relaunch".
    let log = cfg.log.as_ref().and_then(|p| {
        match std::fs::OpenOptions::new().create(true).append(true).open(p) {
            Ok(f) => Some(std::sync::Mutex::new(f)),
            Err(e) => {
                eprintln!("superstar-server: cannot open log {}: {e} (continuing without)", p.display());
                None
            }
        }
    });
    let app = Arc::new(App { assets, api: api::Api::new(defaults, state, profile_codes, prog), log });
    let listener = std::net::TcpListener::bind(&cfg.bind).map_err(|e| format!("bind {}: {e}", cfg.bind))?;
    app.log(&format!(
        "superstar-server listening on {} (public URL {}), {} cards, data {}",
        cfg.bind, cfg.public_url, card_count, cfg.data.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    ));
    http::serve(listener, move |req| handle(&app, req));
    Ok(())
}

fn handle(app: &App, req: &Request) -> Response {
    let path = req.url.split('?').next().unwrap_or("").to_string();
    let get = req.method == "GET" || req.method == "HEAD";

    if get && path == "/health" {
        Response::new(b"ok".to_vec())
    } else if get && path == "/save/full" {
        let body = serde_json::to_vec_pretty(&app.api.export_save()).unwrap_or_default();
        Response::new(body).with_type("application/json")
    } else if (req.method == "PUT" || req.method == "POST") && path == "/save/full" {
        match app.api.import_save(&req.body) {
            Ok(()) => {
                app.log("PUT /save/full -> save replaced");
                Response::new(b"ok, restart the game to see the changes\n".to_vec())
            }
            Err(e) => Response::new(format!("{e}\n").into_bytes()).with_status(400),
        }
    } else if get && path == "/save" {
        let body = serde_json::to_vec_pretty(&app.api.save_summary()).unwrap_or_default();
        Response::new(body).with_type("application/json")
    } else if get && path.contains("/manifest/") {
        app.log(&format!("GET {path} -> manifest"));
        Response::new(app.assets.manifest()).with_type("application/json")
    } else if get && path.starts_with("/cdn/") {
        // Names with non-ASCII characters (e.g. SSGF's "回LABYRINTH.png") arrive percent-encoded.
        match app.assets.cdn_file(&percent_decode(&path["/cdn/".len()..])) {
            Some(bytes) => {
                if std::env::var_os("SUPERSTAR_LOG_CDN").is_some() {
                    app.log(&format!("GET {path} -> {} bytes", bytes.len()));
                }
                Response::new(bytes).with_type(assets::content_type(&path))
            }
            None => {
                app.log(&format!("GET {path} -> 404"));
                Response::new(b"not found".to_vec()).with_status(404)
            }
        }
    } else if req.method == "POST" {
        let reply = app.api.handle(&app.assets, &req.body, req.header("X-SuperStar-AES-IV"));
        app.log(&format!("API {}", reply.log));
        Response::new(reply.body).with_type("application/json")
    } else {
        app.log(&format!("{} {path} -> 404", req.method));
        Response::new(b"not found".to_vec()).with_status(404)
    }
}

/// Decodes %XX escapes (UTF-8); anything malformed is kept as it is.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}
