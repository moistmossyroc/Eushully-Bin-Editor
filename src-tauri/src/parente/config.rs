//! Parente `CONFIG.DAT` codec + language setter.
//!
//! Why: the in-game language button crashes (NullReferenceException in
//! `GT001.ui_allmap.UpdateString` via `ui_config_basic.OnClickBut` — the dev
//! never exercised that path because the buttons were hidden in the JP
//! release), killing the config coroutine and freezing input. The engine
//! honours a SAVED `LangNo` on boot, so we write it straight into CONFIG.DAT.
//! Verified in-game 2026-11-10: LangNo=2 → English dialogue (slot 2).
//!
//! Container (`%APPDATA%\Eushully\YamiyonoPARENTE\Save\CONFIG.DAT`):
//! `[i32 iv_len=16][iv: 16 ASCII chars][i32 len][AES-256-CBC/PKCS7 ciphertext]`.
//! Key = ASCII literal from the game's `AesEncrypt` class (global-metadata).
//! Plaintext = RAW deflate of the engine's saved Lua globals; a number entry is
//! `[u8 name_len][name][u8 type=0x01][4 bytes][f64 value]`.

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use std::io::{Read, Write};

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

const CONFIG_KEY: &[u8; 32] = b"c6eahbq9sjuawhvdr9kvhpsm5qv393ga";
const IV_LEN: usize = 16;
const NUMBER_TYPE_TAG: u8 = 0x01;
/// Bytes between the name and the f64 of a number entry: type tag + 4 bytes.
const NUMBER_VALUE_OFFSET: usize = 1 + 4;
const LANG_GLOBAL: &str = "LangNo";

/// Languages the engine knows (`global_config.txt`: 1 日本語, 2 英語, 3 簡体字, 4 繁体字).
pub const LANG_JAPANESE: u8 = 1;
pub const LANG_ENGLISH: u8 = 2;

/// A decoded CONFIG.DAT: the IV to re-use and the decompressed globals blob.
pub struct DecodedConfig {
    pub iv: [u8; IV_LEN],
    pub plain: Vec<u8>,
}

fn read_i32(data: &[u8], at: usize) -> Result<usize, String> {
    let bytes: [u8; 4] = data
        .get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .ok_or("CONFIG.DAT truncated")?;
    usize::try_from(i32::from_le_bytes(bytes)).map_err(|_| "CONFIG.DAT has a negative length".to_string())
}

/// Pure: decrypt + inflate a CONFIG.DAT file.
pub fn decode_config(file: &[u8]) -> Result<DecodedConfig, String> {
    if read_i32(file, 0)? != IV_LEN {
        return Err("Not a Parente CONFIG.DAT (unexpected IV length)".into());
    }
    let iv: [u8; IV_LEN] = file[4..4 + IV_LEN].try_into().map_err(|_| "CONFIG.DAT truncated")?;
    let cipher_len = read_i32(file, 4 + IV_LEN)?;
    let cipher_start = 8 + IV_LEN;
    let ciphertext = file
        .get(cipher_start..cipher_start + cipher_len)
        .ok_or("CONFIG.DAT ciphertext truncated")?;
    let compressed = Aes256CbcDec::new(CONFIG_KEY.into(), &iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|_| "CONFIG.DAT decryption failed (wrong game or corrupted file)".to_string())?;
    let mut plain = Vec::new();
    flate2::read::DeflateDecoder::new(compressed.as_slice())
        .read_to_end(&mut plain)
        .map_err(|e| format!("CONFIG.DAT decompression failed: {e}"))?;
    Ok(DecodedConfig { iv, plain })
}

/// Pure: deflate + encrypt back into the CONFIG.DAT container.
pub fn encode_config(config: &DecodedConfig) -> Result<Vec<u8>, String> {
    let mut deflater = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    deflater.write_all(&config.plain).map_err(|e| format!("compress: {e}"))?;
    let compressed = deflater.finish().map_err(|e| format!("compress: {e}"))?;
    let ciphertext = Aes256CbcEnc::new(CONFIG_KEY.into(), &config.iv.into())
        .encrypt_padded_vec_mut::<Pkcs7>(&compressed);
    let mut out = Vec::with_capacity(8 + IV_LEN + ciphertext.len());
    out.extend_from_slice(&(IV_LEN as i32).to_le_bytes());
    out.extend_from_slice(&config.iv);
    out.extend_from_slice(&(ciphertext.len() as i32).to_le_bytes());
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Pure: byte offset of the f64 value of a top-level number global.
fn number_value_offset(plain: &[u8], name: &str) -> Option<usize> {
    let name = name.as_bytes();
    (1..plain.len().saturating_sub(name.len() + NUMBER_VALUE_OFFSET + 8)).find_map(|start| {
        let is_entry = plain[start - 1] as usize == name.len()
            && &plain[start..start + name.len()] == name
            && plain[start + name.len()] == NUMBER_TYPE_TAG;
        is_entry.then_some(start + name.len() + NUMBER_VALUE_OFFSET)
    })
}

/// Pure: the saved `LangNo`, if present.
pub fn read_language(plain: &[u8]) -> Option<u8> {
    let at = number_value_offset(plain, LANG_GLOBAL)?;
    let value = f64::from_le_bytes(plain[at..at + 8].try_into().ok()?);
    (value >= 1.0 && value <= 4.0).then_some(value as u8)
}

/// Pure: return a copy of the globals blob with `LangNo` set.
pub fn with_language(plain: &[u8], lang: u8) -> Result<Vec<u8>, String> {
    if !(1..=4).contains(&lang) {
        return Err(format!("Unsupported language {lang} (1=Japanese, 2=English)"));
    }
    let at = number_value_offset(plain, LANG_GLOBAL)
        .ok_or("LangNo not found in CONFIG.DAT — launch the game once, then retry")?;
    let mut out = plain.to_vec();
    out[at..at + 8].copy_from_slice(&f64::from(lang).to_le_bytes());
    Ok(out)
}

/// Default CONFIG.DAT location for the installed game.
pub fn default_config_path() -> Result<std::path::PathBuf, String> {
    let appdata = std::env::var_os("APPDATA").ok_or("APPDATA is not set")?;
    Ok(std::path::Path::new(&appdata).join(r"Eushully\YamiyonoPARENTE\Save\CONFIG.DAT"))
}

/// Read the saved language from a CONFIG.DAT on disk.
pub fn get_language_at(path: &std::path::Path) -> Result<Option<u8>, String> {
    let file = std::fs::read(path).map_err(|e| format!("Can't read {}: {e} — launch the game once first", path.display()))?;
    Ok(read_language(&decode_config(&file)?.plain))
}

/// Set the saved language, verifying the result decodes, after backing up the
/// original once (`CONFIG.DAT.before-language-change`, never overwritten).
/// Returns the backup path.
pub fn set_language_at(path: &std::path::Path, lang: u8) -> Result<std::path::PathBuf, String> {
    let file = std::fs::read(path).map_err(|e| format!("Can't read {}: {e} — launch the game once first", path.display()))?;
    let decoded = decode_config(&file)?;
    let updated = DecodedConfig { iv: decoded.iv, plain: with_language(&decoded.plain, lang)? };
    let encoded = encode_config(&updated)?;
    // Guard: never write something the game couldn't read back.
    if read_language(&decode_config(&encoded)?.plain) != Some(lang) {
        return Err("internal check failed: re-encoded CONFIG.DAT doesn't round-trip".into());
    }
    let backup = path.with_extension("DAT.before-language-change");
    if !backup.exists() {
        std::fs::copy(path, &backup).map_err(|e| format!("Backup failed ({}): {e}", backup.display()))?;
    }
    std::fs::write(path, &encoded).map_err(|e| format!("Write failed ({}): {e}", path.display()))?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_plain(lang: f64) -> Vec<u8> {
        let mut p = vec![0x03, b'F', b'O', b'O', 0x01, 0, 0, 0, 0];
        p.extend_from_slice(&5.0f64.to_le_bytes());
        p.push(LANG_GLOBAL.len() as u8);
        p.extend_from_slice(LANG_GLOBAL.as_bytes());
        p.extend_from_slice(&[0x01, 0, 0, 0, 0]);
        p.extend_from_slice(&lang.to_le_bytes());
        p.extend_from_slice(&[0x09, b'I', b'n', b'p', b'u', b't', b'N', b'a', b'v', b'i', 0x01, 0, 0, 0, 0]);
        p.extend_from_slice(&1.0f64.to_le_bytes());
        p
    }

    #[test]
    fn codec_roundtrips_and_sets_language() {
        let original = DecodedConfig { iv: *b"NX47shPiMMGjX649", plain: synthetic_plain(1.0) };
        let file = encode_config(&original).unwrap();
        let decoded = decode_config(&file).unwrap();
        assert_eq!(decoded.plain, original.plain);
        assert_eq!(read_language(&decoded.plain), Some(LANG_JAPANESE));
        let english = with_language(&decoded.plain, LANG_ENGLISH).unwrap();
        assert_eq!(read_language(&english), Some(LANG_ENGLISH));
        assert_eq!(english.len(), decoded.plain.len(), "only the f64 changes");
    }

    #[test]
    fn rejects_bad_language_and_foreign_files() {
        assert!(with_language(&synthetic_plain(1.0), 9).is_err());
        assert!(decode_config(b"\x04\0\0\0junk").is_err());
    }

    /// Real user CONFIG.DAT copy (machine-local via PARENTE_CONFIG_DAT; skips when unset/absent).
    #[test]
    fn decodes_real_config_copy() {
        let Ok(path) = std::env::var("PARENTE_CONFIG_DAT") else { return };
        let Ok(file) = std::fs::read(path) else { return };
        let decoded = decode_config(&file).unwrap();
        assert_eq!(read_language(&decoded.plain), Some(LANG_JAPANESE));
        let reencoded = encode_config(&DecodedConfig { iv: decoded.iv, plain: with_language(&decoded.plain, LANG_ENGLISH).unwrap() }).unwrap();
        assert_eq!(read_language(&decode_config(&reencoded).unwrap().plain), Some(LANG_ENGLISH));
    }
}
