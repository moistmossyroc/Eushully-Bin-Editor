// external_tool.rs — Locate optional command-line tools (vgmstream, FFmpeg).
//
// Lookup order (first hit wins), same for every tool:
//   1. `<exe dir>/<tool>.exe`               — portable drop-in
//   2. `<exe dir>/<subdir>/<tool>.exe`      — "full" zip layout (keeps DLLs tidy)
//   3. any directory on PATH                — system install
// PATH is the process's own, read at launch: after editing PATH the app
// must be restarted (the UIs say so).

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolSource {
    /// Shipped alongside the application exe.
    Bundled,
    /// Installed system-wide and found on PATH.
    SystemPath,
}

impl ToolSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Bundled => "bundled",
            Self::SystemPath => "PATH",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolLocation {
    pub exe_path: PathBuf,
    pub source: ToolSource,
}

/// Where a tool is looked for, beside the application.
#[derive(Clone, Copy, Debug)]
pub struct ToolSpec {
    /// File name incl. extension, e.g. `ffmpeg.exe`.
    pub exe_name: &'static str,
    /// Folder next to the app exe that the full zip unpacks the tool into.
    pub bundled_subdir: &'static str,
}

/// Bundled candidates for an exe directory, in priority order.
pub fn bundled_candidates(spec: ToolSpec, exe_dir: &Path) -> [PathBuf; 2] {
    [exe_dir.join(spec.exe_name), exe_dir.join(spec.bundled_subdir).join(spec.exe_name)]
}

fn find_bundled(spec: ToolSpec) -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    bundled_candidates(spec, &exe_dir).into_iter().find(|candidate| candidate.is_file())
}

fn find_on_path(spec: ToolSpec) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(spec.exe_name))
        .find(|candidate| candidate.is_file())
}

/// Where the tool lives right now, if anywhere. A few `stat` calls — cheap
/// enough to call per use, so an install mid-session is picked up.
pub fn locate(spec: ToolSpec) -> Option<ToolLocation> {
    find_bundled(spec)
        .map(|exe_path| ToolLocation { exe_path, source: ToolSource::Bundled })
        .or_else(|| find_on_path(spec).map(|exe_path| ToolLocation { exe_path, source: ToolSource::SystemPath }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_candidates_prefer_exe_dir_then_subdir() {
        let spec = ToolSpec { exe_name: "ffmpeg.exe", bundled_subdir: "ffmpeg" };
        let [direct, nested] = bundled_candidates(spec, Path::new("C:/tools"));
        assert_eq!(direct, Path::new("C:/tools/ffmpeg.exe"));
        assert_eq!(nested, Path::new("C:/tools/ffmpeg/ffmpeg.exe"));
    }
}
