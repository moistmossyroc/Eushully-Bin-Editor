// temp_files.rs — Collision-free temp file paths for external-tool round
// trips (vgmstream, FFmpeg). Preview and extraction threads can convert at
// the same time, so a fixed name would let one clobber the other.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `%TEMP%/age_extractor_<tag>_<pid>_<seq>.<extension>`, unique per call.
pub fn unique_temp_path(tag: &str, extension: &str) -> PathBuf {
    let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "age_extractor_{tag}_{}_{sequence}.{extension}",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_paths_differ() {
        assert_ne!(unique_temp_path("t", "wav"), unique_temp_path("t", "wav"));
    }
}
