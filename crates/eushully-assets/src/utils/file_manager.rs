// file_manager.rs — Open a folder (or reveal a file) in the OS file manager.

use std::path::Path;
use std::process::Command;

/// Open `file` with the OS default application for its type (e.g. a movie
/// in the default video player) — the file managers do this for file paths.
pub fn open_with_default_app(file: &Path) -> Result<(), String> {
    open_folder(file)
}

/// Open `folder` in Explorer / Finder / the desktop's file manager.
pub fn open_folder(folder: &Path) -> Result<(), String> {
    let command = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    // Explorer returns exit code 1 even on success, so only spawn errors count.
    Command::new(command)
        .arg(folder)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open {}: {e}", folder.display()))
}
