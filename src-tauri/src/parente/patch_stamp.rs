//! Header timestamps + identity marker for exported patch archives.
//!
//! The game's loader (`Donskoy.Filemapper.AddArchive`, decompiled) loads EVERY
//! `*.arc` under `Archives` recursively — file names are irrelevant — and, per
//! contained file, replaces an already-loaded copy only if
//! `ToUInt64(existing arc timestamp) < ToUInt64(this arc timestamp)` (strict).
//! A tie keeps whichever arc was enumerated first. The timestamp is the ASCII
//! `YYYYMMDDhhmmss` right after the `@ARCH000` magic.
//!
//! So a patch must carry a timestamp NEWER than every archive it overrides —
//! copying a source arc's timestamp (what exports used to do) ties with it and
//! can lose. We stamp the export time (UTC), bumped past the source if needed.

use std::time::{SystemTime, UNIX_EPOCH};

use parente_arc::archive;

/// `YYYYMMDDhhmmss` for a UTC Unix time (proleptic Gregorian, no leap seconds).
pub fn format_archive_timestamp(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let seconds_of_day = unix_seconds % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}{:02}{:02}{:02}",
        seconds_of_day / 3_600,
        (seconds_of_day / 60) % 60,
        seconds_of_day % 60
    )
}

/// Numeric value the engine compares (`Convert.ToUInt64` of the ASCII digits).
/// `None` if the header timestamp isn't all digits.
pub fn archive_timestamp_value(timestamp: &[u8]) -> Option<u64> {
    std::str::from_utf8(timestamp).ok()?.parse().ok()
}

/// Timestamp for a patch built at `unix_seconds` that must outrank every
/// archive whose timestamp is in `source_timestamps`: the export time, or one
/// past the newest source if the clock is behind it.
pub fn patch_timestamp_at(unix_seconds: u64, source_timestamps: &[&[u8]]) -> Vec<u8> {
    let export_time = format_archive_timestamp(unix_seconds);
    let export_value: u64 = export_time.parse().unwrap_or(0);
    let newest_source = source_timestamps
        .iter()
        .filter_map(|t| archive_timestamp_value(t))
        .max()
        .unwrap_or(0);
    if export_value > newest_source {
        export_time.into_bytes()
    } else {
        (newest_source + 1).to_string().into_bytes()
    }
}

/// [`patch_timestamp_at`] for the current time.
pub fn patch_timestamp_now(source_timestamps: &[&[u8]]) -> Vec<u8> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    patch_timestamp_at(now, source_timestamps)
}

/// Directory of the marker entry stamped into every translation patch this
/// editor builds. Plain path (no `!` prefix — the official Append arcs use
/// `/!appNN/`, keep clear of that namespace). Never requested by the game.
pub const MARKER_PATH: &str = "/tlpatch";
/// File name of the marker entry.
pub const MARKER_NAME: &str = "patch_info.txt";
/// Name every pre-marker export used; recognised as ours for compatibility.
const LEGACY_PATCH_FILE_NAME: &str = "ver999.arc";

/// True if `arc` (file `file_name`) is a translation patch built by this
/// editor — such arcs must never serve as a SOURCE for a new export (their
/// files are already translated).
pub fn is_translation_patch(file_name: &str, arc: &archive::ArcArchive) -> bool {
    file_name.eq_ignore_ascii_case(LEGACY_PATCH_FILE_NAME)
        || arc.entries.iter().any(|e| e.path == MARKER_PATH && e.name == MARKER_NAME)
}

/// Stamp, mark and write a translation patch: appends the marker entry, gives
/// the arc a header timestamp newer than every `source_timestamps` entry, and
/// writes it to `out_path`.
pub fn write_patch(
    out_path: &str,
    source_timestamps: &[&[u8]],
    mut files: Vec<archive::PackFile>,
) -> Result<(), String> {
    let timestamp = patch_timestamp_now(source_timestamps);
    files.retain(|f| !(f.path == MARKER_PATH && f.name == MARKER_NAME));
    let info = format!(
        "Translation patch built by Eushully BIN Editor {}\nheader timestamp: {}\nfile: {}\n",
        env!("CARGO_PKG_VERSION"),
        String::from_utf8_lossy(&timestamp),
        std::path::Path::new(out_path).file_name().and_then(|n| n.to_str()).unwrap_or(""),
    );
    files.push(archive::PackFile { name: MARKER_NAME.into(), path: MARKER_PATH.into(), data: info.into_bytes() });
    let packed = archive::pack_arc(&timestamp, &files);
    std::fs::write(out_path, &packed).map_err(|e| format!("Failed to write patch: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        assert_eq!(format_archive_timestamp(0), "19700101000000");
        // 2026-05-19 15:26:11 UTC (ver103_x1's header value).
        assert_eq!(format_archive_timestamp(1_779_204_371), "20260519152611");
        // Leap day.
        assert_eq!(format_archive_timestamp(1_709_164_800), "20240229000000");
    }

    #[test]
    fn export_time_wins_when_newer() {
        let stamp = patch_timestamp_at(1_779_204_371 + 60, &[b"20260519152611"]);
        assert_eq!(stamp, b"20260519152711");
    }

    #[test]
    fn bumps_past_source_when_clock_is_behind() {
        let stamp = patch_timestamp_at(0, &[b"20260501080820", b"20260519152611"]);
        assert_eq!(stamp, b"20260519152612");
    }

    #[test]
    fn recognises_own_patches() {
        let entry = |path: &str, name: &str| archive::ArcEntry { name: name.into(), path: path.into(), offset: 0, size: 0 };
        let marked = archive::ArcArchive { timestamp: vec![], entries: vec![entry(MARKER_PATH, MARKER_NAME)] };
        let official = archive::ArcArchive { timestamp: vec![], entries: vec![entry("/src/sys", "system.txt")] };
        assert!(is_translation_patch("moist English TL v1.0.arc", &marked));
        assert!(is_translation_patch("VER999.ARC", &official));
        assert!(!is_translation_patch("ver103_x1.arc", &official));
    }

    #[test]
    fn ignores_non_numeric_sources() {
        assert_eq!(patch_timestamp_at(0, &[b"garbage"]), b"19700101000000");
    }
}
