// game_discovery.rs — Find game folders below a folder the user picked.
//
// Users often pick the wrong level: the game's install folder instead of
// Parente's `Start`, or a folder holding several games. If `dir` itself
// isn't a game root, search breadth-first (bounded depth and folder count)
// for roots:
//   AGE:     a folder containing SYS3INI / SYS4INI / SYS5INI.BIN
//   Parente: a folder whose `Archives` subfolder holds `.arc` files
//            (that folder is `Start`), or a folder of `.arc` files itself
// A root is never searched inside (its `Archives\Patch` etc. belong to it),
// except a bare folder of `.arc` files (e.g. a translation project folder that
// keeps a built `ver999.arc` next to a full install): it is listed AND searched,
// skipping only its own tier subfolders (Append / Patch).

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_SEARCH_DEPTH: usize = 4;
const MAX_FOLDERS_VISITED: usize = 4_000;
const MAX_ROOTS: usize = 64;
const AGE_INDEX_FILES: [&str; 3] = ["sys5ini.bin", "sys4ini.bin", "sys3ini.bin"];
const PARENTE_ARCHIVES_DIR: &str = "archives";
const SKIPPED_FOLDER_NAMES: [&str; 6] = ["target", "node_modules", ".git", "save", "savedata", "$recycle.bin"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameKind {
    Age,
    Parente,
}

impl GameKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Age => "AGE (SYS*INI.BIN)",
            Self::Parente => "Parente (.arc)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameRoot {
    pub path: PathBuf,
    pub kind: GameKind,
}

pub enum Discovery {
    /// Open this folder (the picked folder itself, or the only game below it).
    Single(GameRoot),
    /// Several games below the picked folder: let the user choose.
    Multiple(Vec<GameRoot>),
    NotFound,
}

struct FolderListing {
    lower_file_names: Vec<String>,
    subfolders: Vec<PathBuf>,
}

fn list_folder(dir: &Path) -> Option<FolderListing> {
    let mut listing = FolderListing { lower_file_names: Vec::new(), subfolders: Vec::new() };
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            listing.subfolders.push(entry.path());
        } else if file_type.is_file() {
            listing.lower_file_names.push(entry.file_name().to_string_lossy().to_lowercase());
        }
    }
    listing.subfolders.sort();
    Some(listing)
}

fn has_arc_files(lower_file_names: &[String]) -> bool {
    lower_file_names.iter().any(|name| name.ends_with(".arc"))
}

/// Parente tier folders that belong to the `.arc` folder they sit in.
const PARENTE_TIER_DIRS: [&str; 2] = ["append", "patch"];

/// A recognised root, and whether its subfolders may still hold other games.
struct Classification {
    kind: GameKind,
    search_inside: bool,
}

fn is_parente_tier_dir(folder: &Path) -> bool {
    folder.file_name().is_some_and(|n| PARENTE_TIER_DIRS.contains(&n.to_string_lossy().to_lowercase().as_str()))
}

fn classify(listing: &FolderListing) -> Option<Classification> {
    if listing.lower_file_names.iter().any(|name| AGE_INDEX_FILES.contains(&name.as_str())) {
        return Some(Classification { kind: GameKind::Age, search_inside: false });
    }
    let archives_dir = listing.subfolders.iter().find(|sub| {
        sub.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(PARENTE_ARCHIVES_DIR))
    });
    let archives_has_arc = archives_dir
        .and_then(|dir| list_folder(dir))
        .is_some_and(|archives| has_arc_files(&archives.lower_file_names));
    if archives_has_arc {
        return Some(Classification { kind: GameKind::Parente, search_inside: false });
    }
    if has_arc_files(&listing.lower_file_names) {
        return Some(Classification { kind: GameKind::Parente, search_inside: true });
    }
    None
}

fn is_skipped(folder: &Path) -> bool {
    folder
        .file_name()
        .is_some_and(|n| SKIPPED_FOLDER_NAMES.contains(&n.to_string_lossy().to_lowercase().as_str()))
}

/// Game roots at or below `dir`, shallowest first.
pub fn find_game_roots(dir: &Path) -> Vec<GameRoot> {
    let mut roots = Vec::new();
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::from([(dir.to_path_buf(), 0)]);
    let mut visited = 0usize;

    while let Some((folder, depth)) = queue.pop_front() {
        visited += 1;
        if visited > MAX_FOLDERS_VISITED || roots.len() >= MAX_ROOTS { break; }
        let Some(listing) = list_folder(&folder) else { continue };
        let classification = classify(&listing);
        let is_bare_arc_folder = classification.as_ref().is_some_and(|found| found.search_inside);
        if let Some(found) = classification {
            roots.push(GameRoot { path: folder, kind: found.kind });
            if !found.search_inside {
                continue; // never search inside a game
            }
        }
        if depth < MAX_SEARCH_DEPTH {
            queue.extend(
                listing
                    .subfolders
                    .into_iter()
                    .filter(|sub| !is_skipped(sub) && !(is_bare_arc_folder && is_parente_tier_dir(sub)))
                    .map(|sub| (sub, depth + 1)),
            );
        }
    }
    roots
}

pub fn discover(dir: &Path) -> Discovery {
    let mut roots = find_game_roots(dir);
    match roots.len() {
        0 => Discovery::NotFound,
        1 => Discovery::Single(roots.remove(0)),
        _ => Discovery::Multiple(roots),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_tree(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("age_discovery_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn touch(path: PathBuf) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
    }

    #[test]
    fn picked_folder_that_is_a_game_is_used_directly() {
        let root = temp_tree("direct");
        touch(root.join("SYS4INI.BIN"));
        touch(root.join("sub/SYS3INI.BIN"));
        assert_eq!(find_game_roots(&root), vec![GameRoot { path: root.clone(), kind: GameKind::Age }]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn finds_parente_start_below_install_folder_and_not_its_archives() {
        let root = temp_tree("parente");
        touch(root.join("Game/Start/Archives/ver00d0.arc"));
        touch(root.join("Game/Start/Archives/Patch/ver999.arc"));
        let found = find_game_roots(&root);
        assert_eq!(found, vec![GameRoot { path: root.join("Game").join("Start"), kind: GameKind::Parente }]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bare_arc_folder_is_listed_and_the_install_inside_it_too() {
        let root = temp_tree("bare_arc");
        touch(root.join("Project/ver999.arc"));
        touch(root.join("Project/Patch/ver999.arc"));
        touch(root.join("Project/Install/Start/Archives/ver00d0.arc"));
        let found: Vec<PathBuf> = find_game_roots(&root).into_iter().map(|r| r.path).collect();
        assert_eq!(found, vec![root.join("Project"), root.join("Project").join("Install").join("Start")]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn multiple_games_are_all_reported() {
        let root = temp_tree("multi");
        touch(root.join("A/SYS4INI.BIN"));
        touch(root.join("B/Start/Archives/x.arc"));
        touch(root.join("C/notes.txt"));
        assert!(matches!(discover(&root), Discovery::Multiple(roots) if roots.len() == 2));
        assert!(matches!(discover(&root.join("C")), Discovery::NotFound));
        let _ = fs::remove_dir_all(root);
    }
}
