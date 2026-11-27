//! Read-only access to server-data/, either as a plain folder (development) or as uncompressed
//! zips: a zip of the folder, or the game's data split APKs, which together hold the tree under
//! `assets/gamedata/` (Android can't verify APKs over 2 GB, so the data is spread over several
//! asset packs; see tools/build-apks.py).
//!
//! Layout (see tools/build-server-data.py):
//!   cdn/<host>/<path>     game files in original CDN URL layout
//!   asset-context.json    master data index
//!   manifest.json         startup manifest template
//!   api-defaults.json     default API results

use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

/// "a//b/" -> "a/b": original URLs contain double slashes (e.g. `asset//<hash>.json.gz`),
/// which a folder tolerates but zip entry names don't.
fn normalize(rel: &str) -> String {
    rel.split('/').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("/")
}

pub enum DataSource {
    Dir(PathBuf),
    /// One or more archives; a file is looked up in each in turn.
    Zip(Vec<ZipPart>),
}

pub struct ZipPart {
    archive: Mutex<zip::ZipArchive<File>>,
    /// Entry name -> index. Names are taken as UTF-8 from the raw bytes: bundletool writes UTF-8
    /// names without the zip's UTF-8 flag, so the zip crate would read them as CP437 and
    /// non-ASCII names (SSGF's "回LABYRINTH.png") wouldn't be found.
    index: std::collections::HashMap<String, usize>,
    /// The folder inside the archive that holds the tree ("" or one of APK_PREFIXES).
    prefix: String,
}

impl ZipPart {
    fn open(path: &Path) -> io::Result<Self> {
        let mut archive = zip::ZipArchive::new(File::open(path)?)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", path.display())))?;
        let mut index = std::collections::HashMap::new();
        for i in 0..archive.len() {
            if let Ok(f) = archive.by_index_raw(i) {
                index.insert(String::from_utf8_lossy(f.name_raw()).into_owned(), i);
            }
        }
        // A data split APK has its files under one of APK_PREFIXES; a plain zip at the top.
        let prefix = APK_PREFIXES
            .iter()
            .find(|p| index.keys().any(|n| n.starts_with(*p)))
            .map_or(String::new(), |p| p.to_string());
        Ok(ZipPart { archive: Mutex::new(archive), index, prefix })
    }
}

/// Where the tree sits inside a data split APK (tools/build-apks.py).
pub const APK_PREFIXES: [&str; 1] = ["assets/gamedata/"];

impl DataSource {
    /// A folder, or one or more zips (the parts of the data split).
    pub fn open(paths: &[PathBuf]) -> io::Result<Self> {
        match paths {
            [dir] if dir.is_dir() => Ok(DataSource::Dir(dir.clone())),
            [] => Err(io::Error::new(io::ErrorKind::NotFound, "no data given")),
            _ => Ok(DataSource::Zip(paths.iter().map(|p| ZipPart::open(p)).collect::<io::Result<_>>()?)),
        }
    }

    /// Reads a file by its relative path (forward slashes). Rejects `..` and absolute paths.
    pub fn read(&self, rel: &str) -> Option<Vec<u8>> {
        let rel = normalize(rel);
        let rel = rel.as_str();
        if Path::new(rel).components().any(|c| !matches!(c, Component::Normal(_))) {
            return None;
        }
        match self {
            DataSource::Dir(root) => std::fs::read(root.join(rel)).ok(),
            DataSource::Zip(parts) => parts.iter().find_map(|part| {
                let i = *part.index.get(&format!("{}{rel}", part.prefix))?;
                let mut archive = part.archive.lock().ok()?;
                let mut entry = archive.by_index(i).ok()?;
                let mut buf = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut buf).ok()?;
                Some(buf)
            }),
        }
    }

    pub fn exists(&self, rel: &str) -> bool {
        let rel = normalize(rel);
        let rel = rel.as_str();
        match self {
            DataSource::Dir(root) => root.join(rel).is_file(),
            DataSource::Zip(parts) => parts.iter().any(|part| part.index.contains_key(&format!("{}{rel}", part.prefix))),
        }
    }

    /// Top-level host folders under cdn/, used to know which URLs to rewrite.
    pub fn cdn_hosts(&self) -> Vec<String> {
        let mut hosts: Vec<String> = match self {
            DataSource::Dir(root) => std::fs::read_dir(root.join("cdn"))
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .filter(|e| e.path().is_dir())
                        .filter_map(|e| e.file_name().into_string().ok())
                        .collect()
                })
                .unwrap_or_default(),
            DataSource::Zip(parts) => parts
                .iter()
                .flat_map(|part| {
                    let cdn = format!("{}cdn/", part.prefix);
                    part.index
                        .keys()
                        .filter_map(|n| n.strip_prefix(cdn.as_str()))
                        .filter_map(|n| n.split('/').next())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .collect(),
        };
        hosts.sort();
        hosts.dedup();
        hosts
    }
}
