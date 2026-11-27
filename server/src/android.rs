//! Android entry: the server as a library inside the game process.
//!
//! The library is shipped as `lib/arm64-v8a/libss.so` and loaded at process start because
//! `libmain.so` (Unity's first native library) lists it as a dependency (see
//! tools/build-apk.py, which renames libmain's `libdl.so` dependency to `libss.so`; this
//! library itself depends on libdl/libm, so nothing goes missing). An ELF constructor
//! starts the server on 127.0.0.1:<port> in a background thread; the port comes from the data's
//! server.json (game.json "port", written by tools/build-server-data.py), default 18384.
//!
//! Paths (PKG = the process name, e.g. com.dalcomsoft.ssom):
//!   data  the installed data splits, <app dir>/split_gamedata0.apk, split_gamedata1.apk, ...
//!         (install-time asset packs, see tools/build-apks.py), tree under assets/gamedata/
//!         or /storage/emulated/0/Android/data/PKG/files/server-data/ (folder, dev)
//!   save  /data/user/0/PKG/files/offline.save.json
//!   log   /storage/emulated/0/Android/data/PKG/files/server.log
//! A save from before the multi-game layout (ssomg.save.json) is renamed.

use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 18384;

/// The game's port from server.json in the data (each game has its own, so several games can
/// run their servers on one phone).
fn port(data: &[PathBuf]) -> u16 {
    crate::data::DataSource::open(data)
        .ok()
        .and_then(|d| d.read("server.json"))
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["port"].as_u64())
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(DEFAULT_PORT)
}

fn process_name() -> String {
    std::fs::read("/proc/self/cmdline")
        .ok()
        .map(|b| String::from_utf8_lossy(b.split(|&c| c == 0).next().unwrap_or(&[])).into_owned())
        .unwrap_or_default()
}

/// Asset packs that hold server-data are named gamedata0, gamedata1, ... (tools/build-apks.py).
/// The package manager installs them next to base.apk as split_<name>.apk.
const DATA_SPLIT: &str = "gamedata";

/// The app's install folder (/data/app/~~X/PKG-Y), from a mapping of one of its files: our
/// own library is loaded from split_config.arm64_v8a.apk, or from lib/arm64/ if extracted.
fn app_dir(pkg: &str) -> Option<PathBuf> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    let marker = format!("/{pkg}-");
    maps.lines()
        .filter_map(|l| l.find("/data/app/").map(|i| &l[i..]))
        .find_map(|path| {
            let start = path.find(&marker)?;
            let end = start + 1 + path[start + 1..].find('/')?;
            Some(PathBuf::from(&path[..end]))
        })
}

/// Unity copies global-metadata.dat to files/il2cpp/ on first start and keeps using that copy
/// until its own build ID changes, so an updated APK (e.g. a new manifest URL, which lives in
/// the metadata for SSGF) would run with the old metadata. Remove the copy when base.apk is
/// newer; Unity extracts it again.
fn drop_stale_metadata(pkg: &str, files: &std::path::Path) {
    let cache = files.join("il2cpp");
    let modified = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let (Some(dir), Some(cached)) = (app_dir(pkg), modified(&cache.join("Metadata/global-metadata.dat"))) else {
        return;
    };
    if modified(&dir.join("base.apk")).is_some_and(|apk| apk > cached) {
        let _ = std::fs::remove_dir_all(&cache);
    }
}

fn find_data(pkg: &str) -> Option<Vec<PathBuf>> {
    if let Some(dir) = app_dir(pkg) {
        let parts: Vec<PathBuf> = (0..)
            .map(|i| dir.join(format!("split_{DATA_SPLIT}{i}.apk")))
            .take_while(|p| p.is_file())
            .collect();
        if !parts.is_empty() {
            return Some(parts);
        }
    }
    let files = format!("/storage/emulated/0/Android/data/{pkg}/files");
    let dir = PathBuf::from(&files).join("server-data");
    dir.is_dir().then(|| vec![dir])
}

fn start() {
    let pkg = process_name();
    // Only the game's main process (not e.g. ":remote" or WebView helper processes).
    if pkg.is_empty() || pkg.contains(':') {
        return;
    }
    let files = PathBuf::from(format!("/storage/emulated/0/Android/data/{pkg}/files"));
    let _ = std::fs::create_dir_all(&files);
    let log = files.join("server.log");
    drop_stale_metadata(&pkg, &files);
    let Some(data) = find_data(&pkg) else {
        let _ = std::fs::write(&log, format!("superstar-server: no data found for {pkg}\n"));
        return;
    };
    // The save kept its first name (ssomg.save.json) until the multi-game layout; move it.
    let app_files = PathBuf::from(format!("/data/user/0/{pkg}/files"));
    let save = app_files.join("offline.save.json");
    let old_save = app_files.join("ssomg.save.json");
    if !save.exists() && old_save.exists() {
        let _ = std::fs::rename(&old_save, &save);
    }
    let port = port(&data);
    let cfg = crate::Config {
        data,
        bind: format!("127.0.0.1:{port}"),
        public_url: format!("http://127.0.0.1:{port}"),
        save,
        log: Some(log.clone()),
    };
    let _ = std::thread::Builder::new().name("superstar-server".into()).spawn(move || {
        if let Err(e) = crate::run(cfg) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
                let _ = writeln!(f, "superstar-server failed: {e}");
            }
        }
    });
}

extern "C" fn on_load() {
    // Never let a panic unwind into the dynamic linker.
    let _ = std::panic::catch_unwind(start);
}

#[used]
#[link_section = ".init_array"]
static INIT: extern "C" fn() = on_load;
