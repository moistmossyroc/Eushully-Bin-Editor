// ffmpeg.rs — Video playback support via the external FFmpeg tool.
//
// The Eushully suite plays movies in a WebView2 `<video>` element, which
// handles H.264/AAC MP4 (Parente's `mp4/*.mp4`) natively but NOT the MPEG-1
// program streams AGE titles ship as `.AGF` (OP/ED/LOGO/MV*). Those are
// transcoded once to a browser-friendly MP4 and cached in %TEMP%.
//
// FFmpeg is located like vgmstream (utils::external_tool): exe dir,
// `ffmpeg/` subdir (full zip), then PATH.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::utils::external_tool::{locate, ToolLocation, ToolSpec};
use crate::utils::temp_files::unique_temp_path;

const FFMPEG: ToolSpec = ToolSpec { exe_name: "ffmpeg.exe", bundled_subdir: "ffmpeg" };
const CACHE_DIR_NAME: &str = "age_extractor_video_cache";
/// Bytes from each end of the file mixed into the cache key (with the length).
const CACHE_KEY_SAMPLE_BYTES: usize = 64 * 1024;
pub const DOWNLOAD_URL: &str = "https://www.gyan.dev/ffmpeg/builds/";

pub fn locate_ffmpeg() -> Option<ToolLocation> {
    locate(FFMPEG)
}

/// Can a WebView2 `<video>` play this container as-is?
pub fn plays_natively_in_browser(extension: &str) -> bool {
    matches!(extension, "mp4" | "webm")
}

/// Stable, cheap cache key: length + head + tail of the data.
fn cache_key(data: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    data.len().hash(&mut hasher);
    data[..data.len().min(CACHE_KEY_SAMPLE_BYTES)].hash(&mut hasher);
    data[data.len().saturating_sub(CACHE_KEY_SAMPLE_BYTES)..].hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn cache_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(CACHE_DIR_NAME);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create video cache: {e}"))?;
    Ok(dir)
}

/// Transcode any FFmpeg-readable video file to H.264/AAC MP4 with the index
/// up front (`+faststart`) so a `<video>` can start and seek immediately.
pub fn transcode_to_browser_mp4(ffmpeg: &Path, source: &Path, destination: &Path) -> Result<(), String> {
    let output = Command::new(ffmpeg)
        .args(["-y", "-v", "error", "-i"])
        .arg(source)
        .args([
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart",
        ])
        .arg(destination)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run {}: {e}", ffmpeg.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let _ = std::fs::remove_file(destination);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("ffmpeg failed: {}", stderr.lines().last().unwrap_or("unknown error")))
}

/// A file the browser can play for `data` (container `extension`), reusing
/// earlier results. Native formats are written as-is; others are
/// transcoded (needs FFmpeg). Safe to call from several threads: results are
/// written to a temp name and renamed into place.
pub fn browser_playable_video(data: &[u8], extension: &str) -> Result<PathBuf, String> {
    let cached = cache_dir()?.join(format!("{}.mp4", cache_key(data)));
    if cached.is_file() {
        return Ok(cached);
    }
    let staging = unique_temp_path("video_out", "mp4");
    if plays_natively_in_browser(extension) {
        std::fs::write(&staging, data).map_err(|e| format!("cannot write video: {e}"))?;
    } else {
        let ffmpeg = locate_ffmpeg().ok_or("FFmpeg not found (needed to convert this movie for playback)")?;
        let source = unique_temp_path("video_in", extension);
        std::fs::write(&source, data).map_err(|e| format!("cannot write video: {e}"))?;
        let result = transcode_to_browser_mp4(&ffmpeg.exe_path, &source, &staging);
        let _ = std::fs::remove_file(&source);
        result?;
    }
    std::fs::rename(&staging, &cached).or_else(|_| {
        // Another thread won the race: keep its file.
        let _ = std::fs::remove_file(&staging);
        if cached.is_file() { Ok(()) } else { Err("cannot move video into cache".to_string()) }
    })?;
    Ok(cached)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_depends_on_content_and_length() {
        assert_eq!(cache_key(b"abc"), cache_key(b"abc"));
        assert_ne!(cache_key(b"abc"), cache_key(b"abd"));
        assert_ne!(cache_key(b"abc"), cache_key(b"abcabc"));
    }

    #[test]
    fn only_mp4_and_webm_play_natively() {
        assert!(plays_natively_in_browser("mp4"));
        assert!(!plays_natively_in_browser("mpg"));
    }

    /// `VIDEO_INPUT=<some .mpg/.AGF movie> cargo test --release real_transcode -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_transcode() {
        let input = PathBuf::from(std::env::var("VIDEO_INPUT").expect("set VIDEO_INPUT"));
        let data = std::fs::read(&input).unwrap();
        let started = std::time::Instant::now();
        let playable = browser_playable_video(&data, "mpg").expect("transcode failed");
        let size = std::fs::metadata(&playable).unwrap().len();
        println!("{} -> {} ({size} bytes) in {:.1?}", input.display(), playable.display(), started.elapsed());
        let again = std::time::Instant::now();
        assert_eq!(browser_playable_video(&data, "mpg").unwrap(), playable);
        println!("cache hit in {:.1?}", again.elapsed());
    }
}
