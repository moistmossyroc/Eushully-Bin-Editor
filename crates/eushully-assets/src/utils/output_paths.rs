// output_paths.rs — Avoid silent overwrites within one extraction job.
//
// Two DIFFERENT entries that map to the same output file (typical with
// "Flatten": `img/ev/a.bytes` and `img/bg/a.bytes` → `a.png`) get `_2`, `_3`…
// The SAME entry name arriving again (another tier's copy, written in load
// order) reuses its path on purpose: the later, effective copy overwrites.
// Paths compare case-insensitively (Windows file systems).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct OutputPathClaims {
    /// lower-cased output path → lower-cased entry name that claimed it
    claimed_by: HashMap<String, String>,
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn numbered_variant(path: &Path, number: usize) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let file_name = match path.extension() {
        Some(extension) => format!("{stem}_{number}.{}", extension.to_string_lossy()),
        None => format!("{stem}_{number}"),
    };
    path.with_file_name(file_name)
}

impl OutputPathClaims {
    /// Final output path for `entry_name`, given the path it would naturally use.
    pub fn claim(&mut self, desired: PathBuf, entry_name: &str) -> PathBuf {
        let entry_key = entry_name.to_lowercase();
        let mut candidate = desired.clone();
        let mut number = 1;
        loop {
            match self.claimed_by.get(&path_key(&candidate)) {
                None => {
                    self.claimed_by.insert(path_key(&candidate), entry_key);
                    return candidate;
                }
                Some(owner) if *owner == entry_key => return candidate,
                Some(_) => {
                    number += 1;
                    candidate = numbered_variant(&desired, number);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_entries_get_numbered_same_entry_overwrites() {
        let mut claims = OutputPathClaims::default();
        let out = PathBuf::from("out/a.png");
        assert_eq!(claims.claim(out.clone(), "img/ev/a.bytes"), PathBuf::from("out/a.png"));
        assert_eq!(claims.claim(out.clone(), "img/bg/a.bytes"), PathBuf::from("out/a_2.png"));
        // Clash detection is case-insensitive; the new name keeps the requested casing.
        let third = claims.claim(PathBuf::from("OUT/A.PNG"), "img/cg/a.bytes");
        assert_eq!(third.to_string_lossy().replace('\\', "/"), "OUT/A_3.PNG");
        assert_eq!(claims.claim(out.clone(), "IMG/EV/a.bytes"), PathBuf::from("out/a.png"));
    }

    #[test]
    fn extensionless_names_number_too() {
        let mut claims = OutputPathClaims::default();
        claims.claim(PathBuf::from("out/abd"), "pfb/abd");
        assert_eq!(claims.claim(PathBuf::from("out/abd"), "pfb/x/abd"), PathBuf::from("out/abd_2"));
    }
}
