use crate::project::types::ProjectFile;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProjectError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Maximum number of rotating backups kept alongside the project JSON.
/// `<name>.json.bak1` is the most recent, `.bak{HISTORY_DEPTH}` the oldest.
const HISTORY_DEPTH: usize = 3;
/// Minimum gap between successive backup rotations. Autosave fires on every
/// row/tab change (could be once per second during active translation), but
/// rotating files that aggressively would (a) churn the disk and (b) blow
/// out the rolling window in seconds. We rotate at most once every 5 minutes
/// of real-edit time so each kept backup represents a meaningful checkpoint.
const ROTATION_INTERVAL_SECS: u64 = 5 * 60;

/// Save a project file to disk as JSON, rotating old versions into a small
/// numbered history (`<name>.json.bak1`..`<name>.json.bak{HISTORY_DEPTH}`).
///
/// Rotation is throttled to once per `ROTATION_INTERVAL_SECS` based on the
/// most recent `.bak1`'s mtime — keeps backups meaningful for the user while
/// not thrashing disk during a translation marathon. The current file is
/// always written; only the rotation step is throttled.
pub fn save_project(path: &Path, project: &ProjectFile) -> Result<(), ProjectError> {
    let json = serde_json::to_string_pretty(project)?;

    if path.exists() {
        if should_rotate_backups(path) {
            // Walk newest-to-oldest, shifting each .bakN -> .bak(N+1).
            // bak{HISTORY_DEPTH} is overwritten / dropped.
            for n in (1..HISTORY_DEPTH).rev() {
                let src = backup_path(path, n);
                let dst = backup_path(path, n + 1);
                if src.exists() {
                    let _ = std::fs::rename(&src, &dst);
                }
            }
            // Move the current file into .bak1.
            let _ = std::fs::rename(path, backup_path(path, 1));
        }
    }

    std::fs::write(path, json)?;
    Ok(())
}

fn backup_path(path: &Path, depth: usize) -> std::path::PathBuf {
    let mut p = path.to_path_buf();
    let base = path
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or("project.json");
    p.set_file_name(format!("{}.bak{}", base, depth));
    p
}

/// Returns true if the most recent backup (`bak1`) is missing or older than
/// the throttle interval — i.e. it's time to rotate.
fn should_rotate_backups(path: &Path) -> bool {
    let bak1 = backup_path(path, 1);
    let Ok(meta) = std::fs::metadata(&bak1) else {
        return true; // no prior backup yet — rotate
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    let Ok(elapsed) = modified.elapsed() else {
        return true;
    };
    elapsed.as_secs() >= ROTATION_INTERVAL_SECS
}

/// Load a project file from disk.
pub fn load_project(path: &Path) -> Result<ProjectFile, ProjectError> {
    let json = std::fs::read_to_string(path)?;
    let project: ProjectFile = serde_json::from_str(&json)?;
    Ok(project)
}
