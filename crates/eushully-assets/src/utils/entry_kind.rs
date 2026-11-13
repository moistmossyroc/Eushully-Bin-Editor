// entry_kind.rs — Coarse file type of an archive entry, from its NAME only
// (the list must classify 15k+ entries without reading any data).
//
// Parente names carry a folder (`img/`, `ogg/`, `src/`, `mp4/`, `pfb/`) whose
// payload type is fixed regardless of the misleading extension. AGE names are
// flat, so the extension decides. Known gap: AGE movies are named `.AGF`
// (OP/ED/LOGO/MV*) and classify as Image until previewed.

use crate::utils::file_magic::name_extension;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Image,
    Audio,
    Script,
    Video,
    Other,
}

impl EntryKind {
    pub const ALL: [EntryKind; 5] = [Self::Image, Self::Audio, Self::Script, Self::Video, Self::Other];

    pub fn label(self) -> &'static str {
        match self {
            Self::Image => "Images",
            Self::Audio => "Audio",
            Self::Script => "Scripts",
            Self::Video => "Video",
            Self::Other => "Other",
        }
    }

    /// Short column text.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Script => "script",
            Self::Video => "video",
            Self::Other => "other",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Image => "🖼",
            Self::Audio => "🔊",
            Self::Script => "📝",
            Self::Video => "🎞",
            Self::Other => "📦",
        }
    }
}

/// Parente top-level folder → kind (payload type is fixed per folder).
const PARENTE_FOLDER_KINDS: &[(&str, EntryKind)] = &[
    ("img/", EntryKind::Image),
    ("ogg/", EntryKind::Audio),
    ("src/", EntryKind::Script),
    ("mp4/", EntryKind::Video),
    ("pfb/", EntryKind::Other),
];

const EXTENSION_KINDS: &[(&[&str], EntryKind)] = &[
    (&["agf", "png", "bmp", "jpg", "jpeg", "tga", "tif", "tiff", "webp"], EntryKind::Image),
    (&["ogg", "wav", "xwm", "xwma", "fsb", "mp3"], EntryKind::Audio),
    (&["bin", "txt", "scr", "aog", "csv", "xml", "json", "ini", "lst"], EntryKind::Script),
    (&["mpg", "mpeg", "mp4", "avi", "wmv"], EntryKind::Video),
];

pub fn classify_entry(entry_name: &str) -> EntryKind {
    let lower = entry_name.to_lowercase();
    if let Some(&(_, kind)) = PARENTE_FOLDER_KINDS.iter().find(|(prefix, _)| lower.starts_with(prefix)) {
        return kind;
    }
    let extension = name_extension(&lower);
    EXTENSION_KINDS
        .iter()
        .find(|(extensions, _)| extensions.contains(&extension.as_str()))
        .map_or(EntryKind::Other, |&(_, kind)| kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parente_folder_beats_misleading_extension() {
        assert_eq!(classify_entry("img/ev/ev157bd.bytes"), EntryKind::Image);
        assert_eq!(classify_entry("ogg/bgm/bgm002.ogg"), EntryKind::Audio);
        assert_eq!(classify_entry("src/game/sc0001.txt"), EntryKind::Script);
        assert_eq!(classify_entry("pfb/ui/ui_title"), EntryKind::Other);
    }

    #[test]
    fn age_names_classify_by_extension() {
        assert_eq!(classify_entry("CS001A.AGF"), EntryKind::Image);
        assert_eq!(classify_entry("BGM01.OGG"), EntryKind::Audio);
        assert_eq!(classify_entry("SC0010.BIN"), EntryKind::Script);
        assert_eq!(classify_entry("OP.mpg"), EntryKind::Video);
        assert_eq!(classify_entry("DATA1.ALF"), EntryKind::Other);
    }
}
