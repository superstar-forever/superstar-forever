//! The game's two AES-256 formats, both keyed with the same 32-byte string.
//!
//! * Master data tables: optional gzip -> base64 -> AES-256-ECB/PKCS7 -> UTF-8 JSON
//!   (`CipherSimple` in the game).
//! * API bodies: base64 -> AES-256-CBC/PKCS7, IV = UTF-8 bytes of the 16-char
//!   `X-SuperStar-AES-IV` header (`Cipher` in the game).

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyInit, KeyIvInit};
use base64::{engine::general_purpose::STANDARD, Engine};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use std::io::{Read, Write};

/// The key from libssnative.so (`ssnative_copys(3549345)`). It isn't in this repository:
/// tools/build-server-data.py finds it in your copy of the game (tools/gamekey.py) and writes it to
/// `server-data/game.key`, which `run()` passes to `set_key`.
static KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();

pub fn set_key(key: &[u8]) -> Result<(), String> {
    let key: [u8; 32] = key.try_into().map_err(|_| format!("game key must be 32 bytes, got {}", key.len()))?;
    let _ = KEY.set(key);
    Ok(())
}

fn key() -> &'static [u8; 32] {
    KEY.get().expect("game key not set")
}

type EcbDec = ecb::Decryptor<aes::Aes256>;
type EcbEnc = ecb::Encryptor<aes::Aes256>;
type CbcDec = cbc::Decryptor<aes::Aes256>;
type CbcEnc = cbc::Encryptor<aes::Aes256>;

#[derive(Debug)]
pub struct CryptoError(pub String);

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> CryptoError + '_ {
    move |e| CryptoError(format!("{what}: {e}"))
}

pub fn decrypt_table(raw: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let mut data = raw.to_vec();
    if raw.starts_with(&[0x1f, 0x8b]) {
        data.clear();
        GzDecoder::new(raw).read_to_end(&mut data).map_err(err("gunzip"))?;
    }
    let bytes = STANDARD.decode(trim_ascii(&data)).map_err(err("base64"))?;
    EcbDec::new(key().into())
        .decrypt_padded_vec_mut::<Pkcs7>(&bytes)
        .map_err(err("aes-ecb"))
}

/// `gzip`: the S3 tables (`*.json.gz`) are gzipped, the URL lists are plain base64.
pub fn encrypt_table(plain: &[u8], gzip: bool) -> Vec<u8> {
    let enc = EcbEnc::new(key().into()).encrypt_padded_vec_mut::<Pkcs7>(plain);
    let b64 = STANDARD.encode(enc);
    if !gzip {
        return b64.into_bytes();
    }
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(b64.as_bytes()).expect("write to Vec");
    gz.finish().expect("finish gzip")
}

fn iv_bytes(iv: &str) -> Result<[u8; 16], CryptoError> {
    iv.as_bytes()
        .try_into()
        .map_err(|_| CryptoError(format!("IV must be 16 bytes, got {}", iv.len())))
}

pub fn decrypt_api(body: &[u8], iv: &str) -> Result<Vec<u8>, CryptoError> {
    let bytes = STANDARD.decode(trim_ascii(body)).map_err(err("base64"))?;
    CbcDec::new(key().into(), &iv_bytes(iv)?.into())
        .decrypt_padded_vec_mut::<Pkcs7>(&bytes)
        .map_err(err("aes-cbc"))
}

pub fn encrypt_api(plain: &[u8], iv: &str) -> Result<Vec<u8>, CryptoError> {
    let enc = CbcEnc::new(key().into(), &iv_bytes(iv)?.into()).encrypt_padded_vec_mut::<Pkcs7>(plain);
    Ok(STANDARD.encode(enc).into_bytes())
}

fn trim_ascii(b: &[u8]) -> &[u8] {
    let start = b.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(b.len());
    let end = b.iter().rposition(|c| !c.is_ascii_whitespace()).map_or(start, |i| i + 1);
    &b[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_roundtrip() {
        let plain = br#"[{"code":1001}]"#;
        assert_eq!(decrypt_table(&encrypt_table(plain, true)).unwrap(), plain);
        assert_eq!(decrypt_table(&encrypt_table(plain, false)).unwrap(), plain);
    }

    #[test]
    fn api_roundtrip() {
        let iv = "abcdefghijklmnop";
        let plain = br#"{"class":"Platform","method":"getTimestamp","params":[]}"#;
        let enc = encrypt_api(plain, iv).unwrap();
        assert_eq!(decrypt_api(&enc, iv).unwrap(), plain);
    }
}
