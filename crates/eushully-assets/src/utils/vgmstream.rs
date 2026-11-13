// vgmstream.rs — FMOD FSB5 → WAV via the external `vgmstream-cli` tool.
//
// FFmpeg can't decode Vorbis-in-FSB5 (the setup headers are stripped and must
// be rebuilt from FMOD's codebook table); vgmstream can. Used by both the
// preview pane and "FSB → WAV" extraction.
//
// Located via utils::external_tool (exe dir, `vgmstream/` subdir, PATH).
// Absent → FSB preview/convert are unavailable and the UI says how to get it.

use std::process::{Command, Stdio};

use crate::utils::external_tool::{locate, ToolSpec};
use crate::utils::temp_files::unique_temp_path;

pub use crate::utils::external_tool::{ToolLocation as VgmstreamLocation, ToolSource as VgmstreamSource};

const VGMSTREAM: ToolSpec = ToolSpec { exe_name: "vgmstream-cli.exe", bundled_subdir: "vgmstream" };
const MIN_WAV_LEN: usize = 44;
pub const DOWNLOAD_URL: &str = "https://vgmstream.org";

/// Where vgmstream-cli lives right now, if anywhere.
pub fn locate_vgmstream() -> Option<VgmstreamLocation> {
    locate(VGMSTREAM)
}

/// Transcode FSB5 bytes to PCM WAV bytes. vgmstream only reads files, so the
/// data round-trips through temp files that are always cleaned up.
pub fn fsb_to_wav(fsb_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let Some(location) = locate_vgmstream() else {
        return Err("vgmstream-cli not found (see the audio tools note in the status bar)".into());
    };
    let in_path = unique_temp_path("fsb", "fsb");
    let out_path = unique_temp_path("fsb", "wav");
    std::fs::write(&in_path, fsb_bytes).map_err(|e| format!("temp write failed: {e}"))?;

    let status = Command::new(&location.exe_path)
        .arg("-o").arg(&out_path)
        .arg(&in_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let result = match status {
        Ok(s) if s.success() => std::fs::read(&out_path)
            .map_err(|e| format!("vgmstream produced no output: {e}")),
        Ok(s) => Err(format!("vgmstream-cli failed ({s})")),
        Err(e) => Err(format!("could not run {}: {e}", location.exe_path.display())),
    };
    let _ = std::fs::remove_file(&in_path);
    let _ = std::fs::remove_file(&out_path);

    result.and_then(|wav| {
        if wav.len() >= MIN_WAV_LEN { Ok(wav) } else { Err("vgmstream output too short".into()) }
    })
}
