//! Startup manifest, master data index and the local CDN.
//!
//! All URLs the game gets from us (manifest, asset context, inside master data tables)
//! are rewritten from the original hosts to `<public_url>/cdn/<host>/...`, so every
//! download goes to this server.

use crate::crypto;
use crate::data::DataSource;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

pub struct Assets {
    data: DataSource,
    public_url: String,
    hosts: Vec<String>,
    /// Rewritten (decrypted, URL-replaced, re-encrypted) table files, by cdn path.
    table_cache: Mutex<HashMap<String, Vec<u8>>>,
}

impl Assets {
    pub fn new(data: DataSource, public_url: &str) -> Self {
        let mut hosts = data.cdn_hosts();
        // Longest first, so e.g. "superstar-<game>.s3..." wins over a shorter suffix match.
        hosts.sort_by_key(|h| std::cmp::Reverse(h.len()));
        Assets {
            data,
            public_url: public_url.trim_end_matches('/').to_string(),
            hosts,
            table_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn data(&self) -> &DataSource {
        &self.data
    }

    pub fn public_url(&self) -> &str {
        &self.public_url
    }

    /// Replaces original CDN/S3 URLs with ours, in plain and JSON-escaped (`\/`) form.
    pub fn rewrite_urls(&self, text: &str) -> String {
        let mut out = text.to_string();
        let public_escaped = self.public_url.replace('/', "\\/");
        for host in &self.hosts {
            for scheme in ["https", "http"] {
                out = out.replace(
                    &format!("{scheme}://{host}/"),
                    &format!("{}/cdn/{host}/", self.public_url),
                );
                out = out.replace(
                    &format!("{scheme}:\\/\\/{host}\\/"),
                    &format!("{public_escaped}\\/cdn\\/{host}\\/"),
                );
            }
        }
        out
    }

    /// The startup manifest (`/version/real/manifest/<version>.txt`).
    pub fn manifest(&self) -> Vec<u8> {
        let mut m: Value = self
            .data
            .read("manifest.json")
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| json!({}));
        let obj = m.as_object_mut().expect("manifest is an object");
        obj.insert("ServerUrl".into(), json!(format!("{}/", self.public_url)));
        obj.insert("Cryption".into(), json!("0")); // plain JSON responses
        obj.insert("MaintenanceUrl".into(), json!("")); // was the end-of-service page
        obj.insert("TesterIp".into(), json!([]));
        obj.insert("DalcomAccountDev".into(), json!("0"));
        self.rewrite_urls(&m.to_string()).into_bytes()
    }

    /// Result of `Platform.checkAssetBundle`: `{version, context: {Table: {file, version}}}`.
    pub fn asset_context(&self) -> Value {
        let raw = self.data.read("asset-context.json").unwrap_or_default();
        let text = String::from_utf8_lossy(&raw);
        let text = text.trim_start_matches('\u{feff}');
        serde_json::from_str(&self.rewrite_urls(text)).unwrap_or_else(|_| json!({}))
    }

    /// Decrypted master data table by name (e.g. "CardData"), via the asset context.
    pub fn table(&self, name: &str) -> Option<Value> {
        let raw = self.data.read("asset-context.json")?;
        let text = String::from_utf8_lossy(&raw);
        let ctx: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
        let url = ctx["context"][name]["file"].as_str()?;
        let path = url.split_once("://")?.1;
        let path = path.split('?').next()?;
        let mut clean = String::new();
        for part in path.split('/').filter(|s| !s.is_empty()) {
            clean.push('/');
            clean.push_str(part);
        }
        let plain = crypto::decrypt_table(&self.data.read(&format!("cdn{clean}"))?).ok()?;
        serde_json::from_slice(&plain).ok()
    }

    /// Serves `cdn/<host>/<path>`. Encrypted master data files get their URLs rewritten.
    pub fn cdn_file(&self, path: &str) -> Option<Vec<u8>> {
        let path = path.trim_start_matches('/');
        let rel = format!("cdn/{path}");
        if is_table_path(path) {
            if let Some(hit) = self.table_cache.lock().ok()?.get(path) {
                return Some(hit.clone());
            }
        }
        let raw = match self.data.read(&rel) {
            Some(b) => b,
            // Most "_Zoom" card frames never existed on the CDN; "_Large" is the closest.
            None if path.contains("_Zoom.") => self
                .data
                .read(&rel.replace("_Zoom.", "_Large."))
                .unwrap_or_else(|| PLACEHOLDER_PNG.to_vec()),
            // Images that are gone from the CDN (old shop/event graphics): the game's launch
            // download refuses to finish if any file fails, so hand out an empty image.
            None if is_image(path) => PLACEHOLDER_PNG.to_vec(),
            None => return None,
        };
        if !is_table_path(path) {
            return Some(raw);
        }
        let out = match crypto::decrypt_table(&raw) {
            Ok(plain) => {
                let text = String::from_utf8_lossy(&plain);
                let gz = raw.starts_with(&[0x1f, 0x8b]);
                crypto::encrypt_table(self.rewrite_urls(&text).as_bytes(), gz)
            }
            Err(_) => raw, // not an encrypted table after all; serve as-is
        };
        if let Ok(mut cache) = self.table_cache.lock() {
            cache.insert(path.to_string(), out.clone());
        }
        Some(out)
    }
}

/// 1x1 fully transparent PNG.
pub const PLACEHOLDER_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

fn is_image(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".png") || p.ends_with(".jpg") || p.ends_with(".jpeg")
}

/// Master data tables (`.../asset/<hash>.json.gz`) and URL lists (`download/real/url/*.json`).
fn is_table_path(path: &str) -> bool {
    path.ends_with(".json.gz") || (path.contains("/download/") && path.ends_with(".json"))
}

pub fn content_type(path: &str) -> &'static str {
    let p = path.split('?').next().unwrap_or(path);
    match p.rsplit('.').next().unwrap_or("") {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "json" | "txt" => "application/json",
        "html" => "text/html; charset=utf-8",
        _ => "application/octet-stream",
    }
}
