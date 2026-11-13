// folder_tree.rs — Folder hierarchy derived from `/`-separated entry names
// (Parente: `img/ev/ev157bd.bytes`). AGE names are flat → tree has no
// children and the UI hides the sidebar.
//
// Folder paths are stored lower-cased with a trailing '/', so "is this entry
// inside folder X" is a `starts_with` on the entry's lower-cased name.

use std::collections::BTreeMap;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FolderNode {
    /// Last path component as first seen, e.g. `ev`. Empty for the root.
    pub display_name: String,
    /// Lower-cased prefix incl. trailing '/', e.g. `img/ev/`. Empty for the root.
    pub path_prefix: String,
    /// Files in this folder and all sub-folders.
    pub file_count: usize,
    pub children: Vec<FolderNode>,
}

impl FolderNode {
    pub fn has_subfolders(&self) -> bool {
        !self.children.is_empty()
    }
}

#[derive(Default)]
struct FolderBuilder {
    display_name: String,
    file_count: usize,
    children: BTreeMap<String, FolderBuilder>,
}

impl FolderBuilder {
    fn into_node(self, path_prefix: String) -> FolderNode {
        let children = self
            .children
            .into_iter()
            .map(|(lower_name, child)| {
                let child_prefix = format!("{path_prefix}{lower_name}/");
                child.into_node(child_prefix)
            })
            .collect();
        FolderNode { display_name: self.display_name, path_prefix, file_count: self.file_count, children }
    }
}

pub fn build_folder_tree<'a>(entry_names: impl Iterator<Item = &'a str>) -> FolderNode {
    let mut root = FolderBuilder::default();
    for name in entry_names {
        root.file_count += 1;
        let mut components: Vec<&str> = name.split('/').filter(|c| !c.is_empty()).collect();
        components.pop(); // file name
        let mut node = &mut root;
        for component in components {
            node = node.children.entry(component.to_lowercase()).or_insert_with(|| FolderBuilder {
                display_name: component.to_string(),
                ..FolderBuilder::default()
            });
            node.file_count += 1;
        }
    }
    root.into_node(String::new())
}

/// `entry_name_lower` lies inside `folder_prefix` (empty prefix = everything).
pub fn is_in_folder(entry_name_lower: &str, folder_prefix: &str) -> bool {
    entry_name_lower.starts_with(folder_prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_nested_counts_case_insensitively() {
        let names = ["img/ev/a.bytes", "img/ev/b.bytes", "IMG/bg/c.bytes", "src/game/s.txt", "assetbundle"];
        let root = build_folder_tree(names.iter().copied());
        assert_eq!(root.file_count, 5);
        let img = &root.children[0];
        assert_eq!((img.display_name.as_str(), img.path_prefix.as_str(), img.file_count), ("img", "img/", 3));
        let subfolders: Vec<&str> = img.children.iter().map(|c| c.path_prefix.as_str()).collect();
        assert_eq!(subfolders, ["img/bg/", "img/ev/"]);
        assert_eq!(root.children[1].path_prefix, "src/");
    }

    #[test]
    fn flat_names_have_no_folders() {
        let root = build_folder_tree(["CS001A.AGF", "BGM01.OGG"].iter().copied());
        assert!(!root.has_subfolders());
        assert!(is_in_folder("cs001a.agf", ""));
        assert!(is_in_folder("img/ev/a.bytes", "img/"));
        assert!(!is_in_folder("img/ev/a.bytes", "src/"));
    }
}
