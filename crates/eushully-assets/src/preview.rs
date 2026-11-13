// preview.rs — File preview: text, image, audio, hex dump.
//
// Runs on a background thread (see app.rs `trigger_preview`). Format decoders
// live in their own modules (agf.rs, utils/vgmstream.rs); this file dispatches
// and turns their output into `PreviewContent`.

use std::sync::{Arc, Mutex};

/// Decoded preview content, produced by a background thread.
#[derive(Clone)]
pub enum PreviewContent {
    None,
    Loading,
    Text(String),
    Image(ImageData),
    Audio(AudioData),
    /// A movie: front-ends play it (suite: <video>) or hand it to a player.
    Video(VideoInfo),
    HexDump(String),
    Error(String),
}

#[derive(Clone)]
pub struct VideoInfo {
    /// Human-readable summary (container, size).
    pub description: String,
    /// Container extension to save it under: "mp4" or "mpg".
    pub extension: &'static str,
}

#[derive(Clone)]
pub struct ImageData {
    pub pixels: Vec<u8>, // RGBA8, row-major, top-down
    pub width: u32,
    pub height: u32,
    /// Source dimensions when the image was downscaled to fit the GPU limit.
    pub original_size: Option<(u32, u32)>,
}

#[derive(Clone)]
pub struct AudioData {
    pub bytes: Vec<u8>,
    pub format: AudioFormat,
    /// Duration in seconds, if it could be determined at decode time.
    pub duration_secs: Option<f32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AudioFormat {
    Wav,
    Ogg,
    Xwm,
    /// FMOD FSB5, transcoded to WAV via vgmstream
    Fsb,
    Unknown,
}

pub type SharedPreview = Arc<Mutex<PreviewContent>>;

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Dispatch on content magic first, file extension second — Parente payloads
/// carry misleading names (PNGs as `.bytes`, FSB5 audio as `.ogg`).
/// Read + decode one archive entry for display. Containers too big to load
/// (lone ALF volumes, huge bundles) only get their prefix described; panics
/// from malformed data become an error. Shared by every front-end.
pub fn preview_entry(archive: &crate::archive::AgeArchive, entry: &crate::archive::FileEntry) -> PreviewContent {
    let is_parente = matches!(entry.source, crate::archive::EntrySource::Parente { .. });
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if needs_prefix_preview(&entry.name, entry.size(), is_parente) {
            return match archive.read_entry_prefix(entry, LARGE_ENTRY_PREVIEW_PREFIX) {
                Ok(prefix) => describe_large_entry(&prefix, entry.size()),
                Err(e) => PreviewContent::Error(format!("Read failed: {e}")),
            };
        }
        match archive.read_entry(entry) {
            Ok(data) => decode_preview(&entry.name, &data),
            Err(e) => PreviewContent::Error(format!("Read failed: {e}")),
        }
    }))
    .unwrap_or_else(|_| PreviewContent::Error("Internal error while decoding (malformed file?)".into()))
}

pub fn decode_preview(filename: &str, data: &[u8]) -> PreviewContent {
    let ext = crate::utils::file_magic::effective_extension(filename, data);
    match ext.as_str() {
        "bmp" | "png" | "jpg" | "jpeg" | "tga" | "webp" => decode_standard_image(data),
        "agf" => decode_agf(data),
        "wav" => decode_audio_wav(data),
        "ogg" => decode_audio_ogg(data),
        "xwm" | "xwma" => decode_audio_xwm(data),
        "fsb" => decode_audio_fsb(data),
        "mp4" => PreviewContent::Video(VideoInfo { description: describe_mp4(data), extension: "mp4" }),
        "mpg" => PreviewContent::Video(VideoInfo { description: describe_mpeg(data), extension: "mpg" }),
        "bundle" => PreviewContent::HexDump(describe_unity_bundle(data)),
        "txt" | "csv" | "xml" | "json" | "ini" | "cfg" | "lst" => decode_text(data),
        "scr" | "aog" => decode_text(data),
        "bin" => decode_bin_strings(data),
        _ => PreviewContent::HexDump(hex_dump(data, 4096)),
    }
}

// ---------------------------------------------------------------------------
// Standard images
// ---------------------------------------------------------------------------

fn decode_standard_image(data: &[u8]) -> PreviewContent {
    use image::ImageReader;
    use std::io::Cursor;

    match ImageReader::new(Cursor::new(data)).with_guessed_format() {
        Ok(reader) => match reader.decode() {
            Ok(img) => rgba_to_preview(img),
            Err(e) => PreviewContent::HexDump(
                format!("Image decode failed: {}\n\n{}", e, hex_dump(data, 512))
            ),
        },
        Err(e) => PreviewContent::HexDump(
            format!("Format detection failed: {}\n\n{}", e, hex_dump(data, 512))
        ),
    }
}

fn rgba_to_preview(img: image::DynamicImage) -> PreviewContent {
    rgba_image_to_preview(img.into_rgba8())
}

fn rgba_image_to_preview(rgba: image::RgbaImage) -> PreviewContent {
    let (width, height) = rgba.dimensions();
    PreviewContent::Image(ImageData { pixels: rgba.into_raw(), width, height, original_size: None })
}

// ---------------------------------------------------------------------------
// AGF (decoder lives in agf.rs, shared with extraction)
// ---------------------------------------------------------------------------

/// AGF preview. Content without an AGF magic (renamed BMP/PNG) falls back to
/// the standard decoders; decode errors show the reason plus a hex dump.
pub fn decode_agf(data: &[u8]) -> PreviewContent {
    if !crate::agf::has_agf_magic(data) {
        return decode_standard_image(data);
    }
    match crate::agf::decode_agf(data) {
        Ok(image) => rgba_image_to_preview(image),
        Err(reason) => PreviewContent::HexDump(format!("AGF decode failed: {reason}\n\n{}", hex_dump(data, 256))),
    }
}

// ---------------------------------------------------------------------------
// Thumbnails (gallery view)
// ---------------------------------------------------------------------------

/// Small RGBA thumbnail (longest side ≤ `max_side`) for image entries;
/// `None` for anything that isn't a decodable image.
pub fn decode_thumbnail(entry_name: &str, data: &[u8], max_side: u32) -> Option<image::RgbaImage> {
    let extension = crate::utils::file_magic::effective_extension(entry_name, data);
    let full = match extension.as_str() {
        "agf" if crate::agf::has_agf_magic(data) => crate::agf::decode_agf(data).ok()?,
        "png" | "bmp" | "jpg" | "jpeg" | "tga" => image::load_from_memory(data).ok()?.into_rgba8(),
        _ => return None,
    };
    let scale = (max_side as f64 / full.width().max(full.height()) as f64).min(1.0);
    let width = ((full.width() as f64 * scale).round() as u32).max(1);
    let height = ((full.height() as f64 * scale).round() as u32).max(1);
    Some(image::imageops::thumbnail(&full, width, height))
}

// ---------------------------------------------------------------------------
// GPU texture limit
// ---------------------------------------------------------------------------

/// Shrink `image` (aspect preserved) so neither side exceeds `max_texture_side`.
/// egui_glow asserts — crashing the whole app — on textures over the GPU limit
/// (8192 on many integrated GPUs), so every preview image passes through here.
pub fn fit_within_texture_limit(image: ImageData, max_texture_side: usize) -> ImageData {
    let max_side = max_texture_side as u32;
    if max_side == 0 || (image.width <= max_side && image.height <= max_side) {
        return image;
    }
    let scale = max_side as f64 / image.width.max(image.height) as f64;
    let target_width = ((image.width as f64 * scale).floor() as u32).clamp(1, max_side);
    let target_height = ((image.height as f64 * scale).floor() as u32).clamp(1, max_side);
    let original_size = image.original_size.unwrap_or((image.width, image.height));
    let Some(source) = image::RgbaImage::from_raw(image.width, image.height, image.pixels) else {
        return ImageData { pixels: Vec::new(), width: 0, height: 0, original_size: None };
    };
    let resized = image::imageops::resize(&source, target_width, target_height, image::imageops::FilterType::Triangle);
    ImageData {
        width: resized.width(),
        height: resized.height(),
        pixels: resized.into_raw(),
        original_size: Some(original_size),
    }
}


// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

fn decode_audio_wav(data: &[u8]) -> PreviewContent {
    let duration_secs = wav_duration(data);
    PreviewContent::Audio(AudioData {
        bytes: data.to_vec(),
        format: AudioFormat::Wav,
        duration_secs,
    })
}

fn decode_audio_ogg(data: &[u8]) -> PreviewContent {
    let duration_secs = ogg_duration_fast(data);
    PreviewContent::Audio(AudioData {
        bytes: data.to_vec(),
        format: AudioFormat::Ogg,
        duration_secs,
    })
}

/// XWM (xWMA) — RIFF container with WMAv2-compressed audio.
/// There is no pure-Rust WMA decoder, so we transcode to WAV via FFmpeg if
/// it is available on PATH. If FFmpeg is absent, we parse the RIFF header and
/// show a readable info dump rather than raw hex.
fn decode_audio_xwm(data: &[u8]) -> PreviewContent {
    // --- Attempt FFmpeg transcoding ---
    if let Some(wav_bytes) = xwm_to_wav_ffmpeg(data) {
        let duration_secs = wav_duration(&wav_bytes);
        return PreviewContent::Audio(AudioData {
            bytes: wav_bytes,
            format: AudioFormat::Xwm,
            duration_secs,
        });
    }

    // --- FFmpeg not available: parse the RIFF header and explain the format ---
    let info = xwm_info(data);
    PreviewContent::HexDump(info)
}

/// Try to convert XWM bytes → PCM WAV bytes using `ffmpeg` on stdin/stdout.
/// Returns None if ffmpeg is not on PATH or conversion fails.
fn xwm_to_wav_ffmpeg(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // FFmpeg can't write a valid WAV to a non-seekable pipe — it would emit
    // 0xFFFFFFFF as the data chunk size sentinel which rodio rejects.
    // Write the output to a temp file so FFmpeg can seek back and fill in
    // the correct chunk sizes, then read the file back into memory.
    let out_path = crate::utils::temp_files::unique_temp_path("xwm_wav", "wav");

    let mut child = Command::new("ffmpeg")
        .args([
            "-v", "quiet",
            "-y",                               // overwrite without prompting
            "-i", "pipe:0",                     // XWM bytes from stdin
            "-f", "wav",
            "-acodec", "pcm_s16le",             // standard PCM output
            out_path.to_str()?,                 // real file → FFmpeg can seek
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    child.stdin.take()?.write_all(data).ok()?;

    let status = child.wait().ok()?;
    if !status.success() { return None; }

    let wav_bytes = std::fs::read(&out_path).ok()?;
    let _ = std::fs::remove_file(&out_path); // clean up

    if wav_bytes.len() < 44 { return None; }
    Some(wav_bytes)
}

/// FMOD FSB5 (Unity AudioClip payload; Parente's `.ogg` entries). Vorbis-in-FSB5
/// needs its setup headers rebuilt, which FFmpeg can't do — see utils::vgmstream.
fn decode_audio_fsb(data: &[u8]) -> PreviewContent {
    match crate::utils::vgmstream::fsb_to_wav(data) {
        Ok(wav_bytes) => PreviewContent::Audio(AudioData {
            duration_secs: wav_duration(&wav_bytes),
            bytes: wav_bytes,
            format: AudioFormat::Fsb,
        }),
        Err(reason) => PreviewContent::HexDump(format!(
            "FMOD FSB5 audio ({}). No preview: {reason}.\n\
             Get vgmstream-cli from https://vgmstream.org (PATH, or next to\n\
             age-extractor.exe). Then tick \"FSB→WAV\" to extract playable .wav files.\n\n{}",
            fsb5_codec_name(data),
            hex_dump(data, 256)
        )),
    }
}

/// FSB5 header: magic(4) version(4) numSamples(4) sampleHeaderSize(4)
/// nameTableSize(4) dataSize(4) mode(4) — `mode` is the codec id.
fn fsb5_codec_name(data: &[u8]) -> &'static str {
    const MODE_OFFSET: usize = 0x18;
    let mode = data
        .get(MODE_OFFSET..MODE_OFFSET + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    match mode {
        Some(1) => "PCM8",
        Some(2) => "PCM16",
        Some(6) => "IMA ADPCM",
        Some(11) => "MPEG",
        Some(15) => "Vorbis",
        Some(_) => "unknown codec",
        None => "truncated header",
    }
}

/// MPEG-1/2 program stream. AGE titles ship their movies (OP, ED, LOGO, MV*)
/// as `.AGF`-named MPEG files; extraction saves them as `.mpg`.
fn describe_mpeg(data: &[u8]) -> String {
    format!(
        "MPEG-1 video, {:.1} MB. The game names it .AGF, but it is a movie\n\
         (extracts as .mpg).",
        data.len() as f64 / (1024.0 * 1024.0)
    )
}

/// Entries bigger than this (non-Parente) preview as a hex dump of their head
/// instead of being read whole; a lone ALF archive can be several GB.
pub const MAX_FULL_PREVIEW_BYTES: u64 = 256 * 1024 * 1024;
pub const LARGE_ENTRY_PREVIEW_PREFIX: u64 = 64 * 1024;

/// True when previewing `entry` must not load all of it.
pub fn needs_prefix_preview(entry_name: &str, stored_size: u64, is_parente: bool) -> bool {
    !is_parente
        && (stored_size > MAX_FULL_PREVIEW_BYTES
            || crate::utils::file_magic::name_extension(entry_name) == "alf")
}

pub fn describe_large_entry(prefix: &[u8], stored_size: u64) -> PreviewContent {
    PreviewContent::HexDump(format!(
        "Large container ({:.1} MB): showing the first {} KB only.\n\
         Extract it to inspect the whole file.\n\n{}",
        stored_size as f64 / (1024.0 * 1024.0),
        prefix.len() / 1024,
        hex_dump(prefix, 4096)
    ))
}

/// One-line MP4 summary for the video preview.
fn describe_mp4(data: &[u8]) -> String {
    let brand = data.get(8..12).map(String::from_utf8_lossy).unwrap_or_default();
    format!(
        "MP4 video — {:.1} MB, brand \"{}\".",
        data.len() as f64 / (1024.0 * 1024.0),
        brand
    )
}

/// Raw UnityFS bundle (prefab/texture/atlas) — not unwrapped by this tool.
fn describe_unity_bundle(data: &[u8]) -> String {
    format!(
        "Unity asset bundle (prefab / textures / sprite atlas).\n\
         Extract it (saved as .bundle) and open in AssetStudio or UABEA.\n\n{}",
        hex_dump(data, 512)
    )
}

/// Parse an XWM RIFF header and return a human-readable description for display
/// when FFmpeg is not available.
fn xwm_info(data: &[u8]) -> String {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return format!(
            "XWM/xWMA audio file — not a valid RIFF/WAVE container\n\n\
             Audio preview requires FFmpeg on PATH for XWM files.\n\n{}",
            hex_dump(data, 256)
        );
    }

    // Walk chunks to find fmt and dpds
    let mut pos = 12usize;
    let mut fmt_tag: Option<u16> = None;
    let mut channels: Option<u16> = None;
    let mut sample_rate: Option<u32> = None;
    let mut byte_rate: Option<u32> = None;
    let mut bits_per_sample: Option<u16> = None;
    let mut data_size: Option<u32> = None;

    while pos + 8 <= data.len() {
        let chunk_id = &data[pos..pos+4];
        let chunk_size = u32::from_le_bytes(data[pos+4..pos+8].try_into().unwrap_or([0;4])) as usize;
        pos += 8;
        let chunk_end = (pos + chunk_size).min(data.len());

        if chunk_id == b"fmt " && chunk_size >= 16 && pos + 16 <= data.len() {
            fmt_tag        = Some(u16::from_le_bytes(data[pos..pos+2].try_into().unwrap_or([0;2])));
            channels       = Some(u16::from_le_bytes(data[pos+2..pos+4].try_into().unwrap_or([0;2])));
            sample_rate    = Some(u32::from_le_bytes(data[pos+4..pos+8].try_into().unwrap_or([0;4])));
            byte_rate      = Some(u32::from_le_bytes(data[pos+8..pos+12].try_into().unwrap_or([0;4])));
            bits_per_sample = Some(u16::from_le_bytes(data[pos+14..pos+16].try_into().unwrap_or([0;2])));
        } else if chunk_id == b"data" {
            data_size = Some(chunk_size as u32);
        }

        pos = chunk_end;
        if chunk_size % 2 != 0 { pos += 1; } // RIFF padding byte
    }

    let codec_name = match fmt_tag {
        Some(0x0161) => "WMAv2 (0x0161)",
        Some(0x0162) => "WMAPro (0x0162)",
        Some(0x0163) => "WMALossless (0x0163)",
        Some(t)      => return format!("XWM: unexpected codec tag 0x{t:04X}\n\n{}", hex_dump(data, 256)),
        None         => "unknown",
    };

    format!(
        "XWM / xWMA Audio\n\
         ─────────────────────────────\n\
         Codec:        {codec_name}\n\
         Channels:     {}\n\
         Sample rate:  {} Hz\n\
         Byte rate:    {} B/s\n\
         Bits/sample:  {}\n\
         Data size:    {}\n\
         File size:    {} bytes\n\
         ─────────────────────────────\n\
         Audio preview requires FFmpeg on PATH.\n\
         Install FFmpeg and restart the application to enable XWM preview.\n\
         Alternatively, extract the file and open it in VLC.",
        channels.map_or("?".into(), |v| v.to_string()),
        sample_rate.map_or("?".into(), |v| v.to_string()),
        byte_rate.map_or("?".into(), |v| v.to_string()),
        bits_per_sample.map_or("?".into(), |v| v.to_string()),
        data_size.map_or("?".into(), |v| format!("{v} bytes")),
        data.len(),
    )
}

/// Determine OGG Vorbis duration without decoding any audio.
///
/// Algorithm:
///   1. Scan forward through pages to find the Vorbis ID header → get sample_rate.
///   2. Scan backward from EOF to find the last page with a non-negative granule → total samples.
///   3. duration = total_samples / sample_rate.
///
/// This reads at most a few KB from the start and end of the file.
fn ogg_duration_fast(data: &[u8]) -> Option<f32> {
    if data.len() < 28 { return None; }

    // ---- Step 1: sample_rate from Vorbis identification header ----
    // Walk OGG pages from the start until we find the Vorbis ID packet.
    let mut sample_rate: Option<u32> = None;
    let mut pos = 0usize;

    'outer: while pos + 27 <= data.len() {
        // Find next OggS page sync
        if &data[pos..pos+4] != b"OggS" {
            pos += 1;
            continue;
        }
        // version must be 0
        if data[pos + 4] != 0 { pos += 1; continue; }

        let n_segs = data[pos + 26] as usize;
        let hdr_end = pos + 27 + n_segs;
        if hdr_end > data.len() { break; }

        let payload_len: usize = data[pos+27..hdr_end].iter().map(|&b| b as usize).sum();
        if hdr_end + payload_len > data.len() { break; }

        let payload = &data[hdr_end..hdr_end + payload_len];

        // Vorbis ID header: packet_type=0x01, "vorbis" (6 bytes)
        // layout: [type:1][vorbis:6][version:4][channels:1][sample_rate:4] ...
        if payload.len() >= 16
            && payload[0] == 0x01
            && &payload[1..7] == b"vorbis"
        {
            sample_rate = Some(u32::from_le_bytes(
                payload[12..16].try_into().ok()?
            ));
            break 'outer;
        }

        pos = hdr_end + payload_len;
    }

    let sr = sample_rate.filter(|&r| r > 0)?;

    // ---- Step 2: last granule from final OGG pages ----
    // Scan backward from EOF looking for "OggS" pages.
    // We cap the scan at the last 65536 bytes — this is always enough for a valid OGG.
    let scan_start = data.len().saturating_sub(65536);
    let mut best_granule: Option<u64> = None;

    let mut i = data.len().saturating_sub(4);
    loop {
        if i < scan_start { break; }
        if i + 14 <= data.len() && &data[i..i+4] == b"OggS" && data[i+4] == 0 {
            // granule_position is i64; -1 (0xFFFFFFFFFFFFFFFF) means "no complete packet"
            let gp = i64::from_le_bytes(data[i+6..i+14].try_into().ok()?);
            if gp >= 0 {
                best_granule = Some(gp as u64);
                break; // first (= last in file) valid granule found
            }
        }
        if i == 0 { break; }
        i -= 1;
    }

    let total_samples = best_granule?;
    Some(total_samples as f32 / sr as f32)
}

/// Quick WAV duration: parse the fmt chunk for sample rate and data chunk for size.
fn wav_duration(data: &[u8]) -> Option<f32> {
    if data.len() < 44 { return None; }
    if &data[0..4] != b"RIFF" { return None; }
    if &data[8..12] != b"WAVE" { return None; }

    let mut pos = 12usize;
    let mut sample_rate: Option<u32> = None;
    let mut num_channels: Option<u16> = None;
    let mut bits_per_sample: Option<u16> = None;
    let mut data_bytes: Option<u32> = None;

    while pos + 8 <= data.len() {
        let chunk_id = &data[pos..pos + 4];
        let chunk_size = u32::from_le_bytes(data[pos+4..pos+8].try_into().ok()?) as usize;
        pos += 8;

        if chunk_id == b"fmt " && chunk_size >= 16 && pos + 16 <= data.len() {
            num_channels    = Some(u16::from_le_bytes(data[pos+2..pos+4].try_into().ok()?));
            sample_rate     = Some(u32::from_le_bytes(data[pos+4..pos+8].try_into().ok()?));
            bits_per_sample = Some(u16::from_le_bytes(data[pos+14..pos+16].try_into().ok()?));
        } else if chunk_id == b"data" {
            // 0xFFFFFFFF is the streaming sentinel written by FFmpeg when the
            // output isn't seekable — reject it so we don't compute a bogus duration.
            if chunk_size != 0xFFFFFFFF {
                data_bytes = Some(chunk_size as u32);
            }
        }

        pos += chunk_size;
        if chunk_size & 1 != 0 { pos += 1; } // RIFF chunks are word-aligned
    }

    let sr  = sample_rate? as f32;
    let ch  = num_channels? as f32;
    let bps = bits_per_sample? as f32;
    let db  = data_bytes? as f32;

    if sr == 0.0 || ch == 0.0 || bps == 0.0 { return None; }
    Some(db / (sr * ch * (bps / 8.0)))
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// Decode a .bin script file by extracting human-readable strings from it.
///
/// AGE engine .bin files are compiled scripts: they contain binary opcodes,
/// offsets, and other non-text data interspersed with Shift-JIS dialogue and
/// label strings.  Attempting to decode the whole file as text produces mostly
/// garbage (triggering the hex-dump fallback).  Instead we scan the raw bytes
/// and pull out every run of valid Shift-JIS / ASCII text that is long enough
/// to be meaningful, mirroring what the Unix `strings` utility does but tuned
/// for Shift-JIS encoded content.
fn decode_bin_strings(data: &[u8]) -> PreviewContent {
    use encoding_rs::SHIFT_JIS;

    // Minimum byte run length to keep.  Short runs are usually padding or
    // coincidental byte sequences that happen to be valid SJIS.
    const MIN_RUN: usize = 3;
    // Maximum characters we will show in the preview.
    const MAX_CHARS: usize = 16384;

    let mut out = String::new();
    let mut out_char_count = 0usize;
    let mut i = 0usize;

    while i < data.len() && out_char_count < MAX_CHARS {
        // --- Try to consume a run of valid Shift-JIS bytes starting at i ---
        let start = i;
        let mut j = i;

        while j < data.len() {
            let b = data[j];

            // Single-byte ASCII printable (0x20–0x7E) or common whitespace
            if (0x20..=0x7E).contains(&b) || b == b'\n' || b == b'\r' || b == b'\t' {
                j += 1;
                continue;
            }

            // Shift-JIS lead byte ranges:
            //   0x81–0x9F  (JIS X 0208 plane 1)
            //   0xE0–0xEA  (JIS X 0208 plane 2)
            if (0x81..=0x9F).contains(&b) || (0xE0..=0xEA).contains(&b) {
                // Need a valid trail byte
                if j + 1 < data.len() {
                    let t = data[j + 1];
                    if (0x40..=0x7E).contains(&t) || (0x80..=0xFC).contains(&t) {
                        j += 2;
                        continue;
                    }
                }
                // Invalid / truncated double-byte — end of run
                break;
            }

            // Half-width katakana (0xA1–0xDF) — single byte, valid SJIS
            if (0xA1..=0xDF).contains(&b) {
                j += 1;
                continue;
            }

            // Anything else is binary — end the run
            break;
        }

        let run_len = j - start;
        if run_len >= MIN_RUN {
            // Decode the captured run through SHIFT_JIS to get proper Unicode
            let (decoded, _, _) = SHIFT_JIS.decode(&data[start..j]);
            // Strip control chars except ordinary whitespace
            let clean: String = decoded
                .chars()
                .filter(|&c| !c.is_control() || c == '\n' || c == '\r' || c == '\t')
                .collect();
            if !clean.trim().is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                    out_char_count += 1;
                }
                out_char_count += clean.chars().count();
                out.push_str(&clean);
            }
        }

        // Advance past the run (or skip one binary byte if no run started)
        i = if run_len > 0 { j } else { i + 1 };
    }

    if out.trim().is_empty() {
        // No readable strings found — fall back to text decode so the caller
        // still gets something useful (which may itself fall back to hex dump).
        return decode_text(data);
    }

    PreviewContent::Text(out.chars().take(MAX_CHARS).collect())
}

/// Characters shown in the text preview (the diff view decodes whole files).
const TEXT_PREVIEW_MAX_CHARS: usize = 200_000;

/// Whole-file text decode: UTF-8, UTF-16LE with BOM, then Shift-JIS.
/// `None` when the bytes look binary.
pub fn decode_text_content(data: &[u8]) -> Option<String> {
    if let Ok(s) = std::str::from_utf8(data) {
        return Some(s.trim_start_matches('\u{FEFF}').to_string());
    }
    if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xFE {
        let u16s: Vec<u16> = data[2..].chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return Some(String::from_utf16_lossy(&u16s));
    }
    use encoding_rs::SHIFT_JIS;
    let (decoded, _, had_errors) = SHIFT_JIS.decode(data);
    let ctrl_count = decoded.chars()
        .filter(|&c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
        .count();
    (!had_errors || ctrl_count < 10).then(|| decoded.into_owned())
}

fn decode_text(data: &[u8]) -> PreviewContent {
    match decode_text_content(data) {
        Some(text) => PreviewContent::Text(text.chars().take(TEXT_PREVIEW_MAX_CHARS).collect()),
        None => PreviewContent::HexDump(hex_dump(data, 4096)),
    }
}

// ---------------------------------------------------------------------------
// Hex dump
// ---------------------------------------------------------------------------

pub fn hex_dump(data: &[u8], max_bytes: usize) -> String {
    let truncated = data.len() > max_bytes;
    let show = &data[..data.len().min(max_bytes)];
    let mut out = String::with_capacity(show.len() * 4);
    for (i, chunk) in show.chunks(16).enumerate() {
        out.push_str(&format!("{:08X}  ", i * 16));
        for (j, b) in chunk.iter().enumerate() {
            out.push_str(&format!("{:02X} ", b));
            if j == 7 { out.push(' '); }
        }
        if chunk.len() < 16 {
            let pad = (16 - chunk.len()) * 3 + if chunk.len() <= 8 { 1 } else { 0 };
            for _ in 0..pad { out.push(' '); }
        }
        out.push_str("  ");
        for b in chunk {
            out.push(if b.is_ascii_graphic() || *b == b' ' { *b as char } else { '.' });
        }
        out.push('\n');
    }
    if truncated {
        out.push_str(&format!("\n… truncated, showing {max_bytes} of {} bytes", data.len()));
    }
    out
}

#[cfg(test)]
mod image_audit {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::Path;

    fn u32_at(data: &[u8], offset: usize) -> Option<usize> {
        data.get(offset..offset + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    }

    /// (bpp, has ACIF after the pixel block), parsed independently of decode_agf.
    fn agf_layout(data: &[u8]) -> Option<(u16, bool)> {
        let hdr_uncomp = u32_at(data, 0x10)?;
        let hdr_comp = u32_at(data, 0x14)?;
        let (hdr_block, hdr_end) = if hdr_comp < hdr_uncomp {
            (crate::lzss::decompress(data.get(0x18..0x18 + hdr_comp)?, 0).ok()?, 0x18 + hdr_comp)
        } else {
            (data.get(0x18..0x18 + hdr_uncomp)?.to_vec(), 0x18 + hdr_uncomp)
        };
        let bpp = u16::from_le_bytes(hdr_block.get(0x10 + 14..0x10 + 16)?.try_into().ok()?);
        let px_end = hdr_end + 12 + u32_at(data, hdr_end + 8)?;
        Some((bpp, data.get(px_end..px_end + 4) == Some(b"ACIF")))
    }

    /// ACIF alpha plane after the pixel block, decompressed (audit-only parse).
    fn acif_alpha(data: &[u8]) -> Option<Vec<u8>> {
        let hdr_uncomp = u32_at(data, 0x10)?;
        let hdr_comp = u32_at(data, 0x14)?;
        let hdr_end = 0x18 + if hdr_comp < hdr_uncomp { hdr_comp } else { hdr_uncomp };
        let acif = hdr_end + 12 + u32_at(data, hdr_end + 8)?;
        if data.get(acif..acif + 4) != Some(b"ACIF") { return None; }
        let alpha_comp = u32_at(data, acif + 0x20)?;
        crate::lzss::decompress(data.get(acif + 0x24..acif + 0x24 + alpha_comp)?, 0).ok()
    }

    /// How "pure" the transparent region is: share of alpha==0 pixels that
    /// carry the single most common RGB among them. The correct alpha row
    /// order lines transparency up with the flat fill colour, so it scores higher.
    fn transparent_fill_purity(rgba: &[u8], width: usize, height: usize, flip_alpha_rows: bool) -> Option<f64> {
        let mut colour_counts: std::collections::HashMap<[u8; 3], usize> = Default::default();
        let mut transparent_total = 0usize;
        for row in 0..height {
            let alpha_row = if flip_alpha_rows { height - 1 - row } else { row };
            for col in 0..width {
                let alpha = rgba[(alpha_row * width + col) * 4 + 3];
                if alpha != 0 { continue; }
                let p = (row * width + col) * 4;
                *colour_counts.entry([rgba[p], rgba[p + 1], rgba[p + 2]]).or_default() += 1;
                transparent_total += 1;
            }
        }
        let top = colour_counts.values().max().copied()?;
        Some(top as f64 / transparent_total as f64)
    }

    /// `AGE_DIR=<dir> AGE_DUMP=<entry name> AGE_DUMP_OUT=<dir>`: writes the entry as
    /// decoded (alpha as-is) and with alpha rows flipped, composited on magenta.
    #[test]
    #[ignore]
    fn dump_agf_alpha_variants() {
        let dir = std::env::var("AGE_DIR").expect("set AGE_DIR");
        let wanted = std::env::var("AGE_DUMP").expect("set AGE_DUMP");
        let out_dir = std::path::PathBuf::from(std::env::var("AGE_DUMP_OUT").expect("set AGE_DUMP_OUT"));
        let archive = crate::archive::AgeArchive::open(Path::new(&dir)).expect("open failed");
        for name in wanted.split(',') {
            let entry = archive.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name)).expect("entry not found");
            let data = archive.read_entry(entry).unwrap();
            let PreviewContent::Image(mut img) = decode_agf(&data) else { panic!("not an image") };
            let (w, h) = (img.width as usize, img.height as usize);
            if let (Some((24, true)), Some(alpha)) = (agf_layout(&data), acif_alpha(&data)) {
                for (i, px) in img.pixels.chunks_exact_mut(4).enumerate() { px[3] = alpha[i]; }
            }
            for flip in [false, true] {
                let mut composited = vec![0u8; w * h * 3];
                for row in 0..h {
                    let alpha_row = if flip { h - 1 - row } else { row };
                    for col in 0..w {
                        let p = (row * w + col) * 4;
                        let alpha = img.pixels[(alpha_row * w + col) * 4 + 3] as u32;
                        let backdrop = [255u32, 0, 255];
                        for c in 0..3 {
                            composited[(row * w + col) * 3 + c] =
                                ((img.pixels[p + c] as u32 * alpha + backdrop[c] * (255 - alpha)) / 255) as u8;
                        }
                    }
                }
                let suffix = if flip { "flipped" } else { "current" };
                image::RgbImage::from_raw(w as u32, h as u32, composited).unwrap()
                    .save(out_dir.join(format!("{name}_{suffix}.png"))).unwrap();
            }
        }
    }

    /// `AGE_DIR=<game dir> cargo test --release image_audit -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_image_audit() {
        let dir = std::env::var("AGE_DIR").expect("set AGE_DIR");
        let archive = crate::archive::AgeArchive::open(Path::new(&dir)).expect("open failed");
        let mut outcomes: BTreeMap<String, usize> = BTreeMap::new();
        let mut examples: BTreeMap<String, String> = BTreeMap::new();
        let (mut max_w, mut max_h) = (0u32, 0u32);
        let (mut alpha_current_wins, mut alpha_flipped_wins, mut alpha_ties) = (0, 0, 0);

        let mut record = |key: String, name: &str| {
            *outcomes.entry(key.clone()).or_default() += 1;
            examples.entry(key).or_insert_with(|| name.to_string());
        };

        for entry in &archive.entries {
            let name_ext = crate::utils::file_magic::name_extension(&entry.name);
            let is_parente_img = entry.name.starts_with("img/");
            if name_ext != "agf" && !is_parente_img && !["png", "bmp", "jpg"].contains(&name_ext.as_str()) {
                continue;
            }
            let Ok(data) = archive.read_entry(entry) else { record("read error".into(), &entry.name); continue };

            if name_ext != "agf" {
                match image::ImageReader::new(std::io::Cursor::new(&data)).with_guessed_format().ok().and_then(|r| r.into_dimensions().ok()) {
                    Some((w, h)) => { max_w = max_w.max(w); max_h = max_h.max(h); record("std ok".into(), &entry.name); }
                    None => record("std dims failed".into(), &entry.name),
                }
                continue;
            }

            if let Some((bpp, has_acif)) = agf_layout(&data) {
                record(format!("agf layout bpp={bpp} acif={has_acif}"), &entry.name);
            }
            let decoded = std::panic::catch_unwind(|| decode_agf(&data));
            match decoded {
                Err(_) => record("agf PANIC".into(), &entry.name),
                Ok(PreviewContent::Image(img)) => {
                    max_w = max_w.max(img.width);
                    max_h = max_h.max(img.height);
                    record("agf image".into(), &entry.name);
                    let (w, h) = (img.width as usize, img.height as usize);
                    if let (Some((24, true)), Some(alpha)) = (agf_layout(&data), acif_alpha(&data)) {
                        if alpha.len() >= w * h {
                            let mut with_alpha = img.pixels.clone();
                            for (i, px) in with_alpha.chunks_exact_mut(4).enumerate() { px[3] = alpha[i]; }
                            let current = transparent_fill_purity(&with_alpha, w, h, false).unwrap_or(0.0);
                            let flipped = transparent_fill_purity(&with_alpha, w, h, true).unwrap_or(0.0);
                            let verdict = if (current - flipped).abs() < 0.02 { "tie" } else if current > flipped { "top-down" } else { "bottom-up" };
                            record(format!("24bpp ACIF alpha order: {verdict}"), &entry.name);
                        } else {
                            record(format!("24bpp ACIF alpha short ({} < {})", alpha.len(), w * h), &entry.name);
                        }
                    }
                    let has_transparency = img.pixels.chunks_exact(4).any(|p| p[3] == 0);
                    if has_transparency {
                        let current = transparent_fill_purity(&img.pixels, w, h, false).unwrap_or(0.0);
                        let flipped = transparent_fill_purity(&img.pixels, w, h, true).unwrap_or(0.0);
                        if (current - flipped).abs() < 0.02 { alpha_ties += 1 }
                        else if current > flipped { alpha_current_wins += 1 }
                        else { alpha_flipped_wins += 1; record("alpha flipped better".into(), &entry.name); }
                    }
                }
                Ok(PreviewContent::HexDump(msg)) => {
                    println!("  FAIL {} magic {:02X?}", entry.name, &data[..data.len().min(16)]);
                    let reason: String = msg.lines().next().unwrap_or("").chars().take(60).collect();
                    record(format!("agf fail: {reason}"), &entry.name);
                }
                Ok(_) => record("agf other".into(), &entry.name),
            }
        }

        println!("== {} ==", dir);
        for (key, count) in &outcomes {
            println!("{count:>7}  {key:<70} e.g. {}", examples.get(key).map(String::as_str).unwrap_or(""));
        }
        println!("max image dims: {max_w}x{max_h}");
        println!("alpha row order: current better {alpha_current_wins}, flipped better {alpha_flipped_wins}, ties {alpha_ties}");
        if let Some(example) = examples.get("alpha flipped better") { println!("  flipped example: {example}"); }
    }
}

#[cfg(test)]
mod texture_limit_tests {
    use super::*;

    fn solid(width: u32, height: u32) -> ImageData {
        ImageData { pixels: vec![200; (width * height * 4) as usize], width, height, original_size: None }
    }

    #[test]
    fn small_images_pass_through_untouched() {
        let fitted = fit_within_texture_limit(solid(100, 50), 2048);
        assert_eq!((fitted.width, fitted.height, fitted.original_size), (100, 50, None));
    }

    #[test]
    fn oversized_images_shrink_keeping_aspect_and_remember_source_size() {
        let fitted = fit_within_texture_limit(solid(400, 100), 200);
        assert_eq!((fitted.width, fitted.height), (200, 50));
        assert_eq!(fitted.original_size, Some((400, 100)));
        assert_eq!(fitted.pixels.len(), 200 * 50 * 4);
    }
}
