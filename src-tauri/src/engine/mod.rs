//! Engine-aware tooling for installed SYS3/SYS4 games: catalog inspection and
//! fanpatch branding (title badge + boot splash). Knowledge base:
//! docs/engine/age-sys.md.

pub mod box_art;
pub mod box_profile;
pub mod dev_title;
pub mod text_metrics;
pub mod branding;
pub mod catalog;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::agf::{encode_agf, AgfImage};
use crate::alf;
use crate::bin_format::parser::parse_bin;
use crate::bin_format::writer::write_bin;
use branding::{BadgePlan, SplashPlan};

/// File name of the branding fanpatch. Sorts after translation fanpatches
/// (`01_…`) so the loader serves our TITLE/LOGO/catalog over theirs.
pub const BRANDING_FANPATCH: &str = "zz_branding.fanpatch";
const BADGE_RECORD: &str = "FPBADGE.AGF";
const SPLASH_RECORD: &str = "FPSPLASH.AGF";

/// What branding a game supports, for the UI (sizes to render art at).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrandingProfile {
    pub title: String,
    pub build: String,
    pub screen_width: u32,
    pub screen_height: u32,
    pub badge: BadgePlan,
    pub splash: SplashPlan,
}

fn master_path(game_dir: &Path) -> Result<PathBuf, String> {
    ["SYS4INI.BIN", "SYS3INI.BIN", "SYS5INI.BIN"]
        .iter()
        .map(|name| game_dir.join(name))
        .find(|p| p.exists())
        .ok_or_else(|| format!("{} has no SYS4INI.BIN/SYS3INI.BIN (pick the folder with AGE.EXE)", game_dir.display()))
}

/// `.fanpatch` files the loader would read, alphabetical (last wins), minus ours.
fn other_fanpatches(game_dir: &Path) -> Vec<PathBuf> {
    let mut patches: Vec<PathBuf> = std::fs::read_dir(game_dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("fanpatch")).unwrap_or(false))
                .filter(|p| p.file_name().map(|n| n != BRANDING_FANPATCH).unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    patches.sort();
    patches
}

fn read_from_fanpatch(patch: &Path, name: &str) -> Option<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(patch).ok()?).ok()?;
    let index = (0..archive.len()).find(|&i| archive.by_index(i).map(|f| f.name().eq_ignore_ascii_case(name)).unwrap_or(false))?;
    let mut file = archive.by_index(index).ok()?;
    let mut bytes = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// The bytes the game would load for `name`, in the loader's priority order:
/// other fanpatches (last alphabetical wins) → loose file → catalog archive.
fn effective_file(game_dir: &Path, master: &alf::AlfIndex, name: &str) -> Result<Vec<u8>, String> {
    if let Some(bytes) = other_fanpatches(game_dir).iter().rev().find_map(|p| read_from_fanpatch(p, name)) {
        return Ok(bytes);
    }
    let entry = master.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name)).ok_or_else(|| format!("{name} not in the catalog"))?;
    let loose = game_dir.join(&entry.name);
    if loose.exists() {
        return std::fs::read(&loose).map_err(|e| format!("read {}: {e}", loose.display()));
    }
    alf::extract_entry(game_dir, master, entry).map_err(|e| format!("extract {name}: {e}"))
}

/// The game's own copy of `name` (loose → archive), ignoring every `.fanpatch` — the base
/// that patches are built from, so re-exports never patch an already-patched file.
fn base_file(game_dir: &Path, master: &alf::AlfIndex, name: &str) -> Result<Vec<u8>, String> {
    let entry = master.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name)).ok_or_else(|| format!("{name} not in the catalog"))?;
    let loose = game_dir.join(&entry.name);
    if loose.exists() {
        return std::fs::read(&loose).map_err(|e| format!("read {}: {e}", loose.display()));
    }
    alf::extract_entry(game_dir, master, entry).map_err(|e| format!("extract {name}: {e}"))
}

/// The native body box from the game's base system script (no `.fanpatch` applied).
pub fn base_box_geometry(game_dir: &Path) -> Result<box_profile::BoxGeometry, String> {
    let master = alf::read_sys4_master(&std::fs::read(master_path(game_dir)?).map_err(|e| format!("read catalog: {e}"))?)
        .ok_or("failed to parse the master catalog")?;
    let system = SYSTEM_SCRIPTS
        .iter()
        .find_map(|name| base_file(game_dir, &master, name).ok())
        .ok_or("no SYSTEM3/4/5.BIN in this game")?;
    let bin = parse_bin(&system).map_err(|e| format!("parse system script: {e}"))?;
    box_profile::detect_geometry(&bin.instructions).ok_or_else(|| "the system script defines no text layout slot 1".to_string())
}

/// A box profile applied to a game, ready to bundle into a translation `.fanpatch`.
pub struct BoxProfilePatch {
    /// The native body box the profile was computed from (base game files).
    pub geometry: box_profile::BoxGeometry,
    /// Patched SYSTEM script (`SYSTEM4.BIN` …) and every other scene whose ADV spacing
    /// changed, as (request name, bytes). Names in `exclude` are skipped (exported
    /// translated scenes are patched by `apply_box_profile_to_scene` instead).
    pub files: Vec<(String, Vec<u8>)>,
}

/// Apply `profile` to the installed game's base files: the system script (origin, height,
/// spacing) and the ADV spacing every dialogue scene re-asserts at its start — scenes left
/// unpatched would fall back to the native spacing and clip the extra line.
pub fn box_profile_patch(
    game_dir: &Path,
    profile: &box_profile::BoxProfile,
    exclude: &std::collections::HashSet<String>,
) -> Result<BoxProfilePatch, String> {
    let master = alf::read_sys4_master(&std::fs::read(master_path(game_dir)?).map_err(|e| format!("read catalog: {e}"))?)
        .ok_or("failed to parse the master catalog")?;
    let (system_name, system) = SYSTEM_SCRIPTS
        .iter()
        .find_map(|name| base_file(game_dir, &master, name).ok().map(|bytes| (name.to_string(), bytes)))
        .ok_or("no SYSTEM3/4/5.BIN in this game")?;
    let system_bin = parse_bin(&system).map_err(|e| format!("parse {system_name}: {e}"))?;
    let geometry = box_profile::detect_geometry(&system_bin.instructions).ok_or("the system script defines no text layout slot 1")?;
    let mut files = Vec::new();
    if !exclude.contains(&system_name) {
        let mut instructions = system_bin.instructions.clone();
        box_profile::patch_system_script(&mut instructions, &geometry, profile);
        files.push((system_name.clone(), write_bin(&system_bin.header, &instructions).map_err(|e| format!("write {system_name}: {e:?}"))?));
    }
    for entry in master.entries.iter().filter(|e| e.name.to_ascii_uppercase().ends_with(".BIN")) {
        let name = entry.name.to_ascii_uppercase();
        if name == system_name || exclude.contains(&name) {
            continue;
        }
        let Ok(bytes) = base_file(game_dir, &master, &entry.name) else { continue };
        let Ok(bin) = parse_bin(&bytes) else { continue };
        let mut instructions = bin.instructions.clone();
        if box_profile::patch_scene_script(&mut instructions, &geometry, profile) == 0 {
            continue;
        }
        files.push((name, write_bin(&bin.header, &instructions).map_err(|e| format!("write {}: {e:?}", entry.name))?));
    }
    Ok(BoxProfilePatch { geometry, files })
}

/// Developer scene jump for a testing build: the game's `TITLE.BIN` (or `title`, an exported
/// one) with the developer menu unlocked behind the Quit button (`dev_title`). `Ok(None)`
/// when the title has no reachable menu pattern (SYS3, unknown builds).
pub fn dev_title_patch(game_dir: &Path, title: Option<&[u8]>) -> Result<Option<Vec<u8>>, String> {
    let master = alf::read_sys4_master(&std::fs::read(master_path(game_dir)?).map_err(|e| format!("read catalog: {e}"))?)
        .ok_or("failed to parse the master catalog")?;
    let bytes = match title {
        Some(bytes) => bytes.to_vec(),
        None => base_file(game_dir, &master, "TITLE.BIN")?,
    };
    let bin = parse_bin(&bytes).map_err(|e| format!("parse TITLE.BIN: {e}"))?;
    let script_exits = |id: u32| {
        master
            .entries
            .get(id as usize)
            .and_then(|entry| base_file(game_dir, &master, &entry.name).ok())
            .and_then(|script| parse_bin(&script).ok())
            .is_some_and(|script| dev_title::ends_with_exit(&script.instructions))
    };
    let mut instructions = bin.instructions.clone();
    if dev_title::unlock_debug_menu(&mut instructions, script_exits).is_none() {
        return Ok(None);
    }
    write_bin(&bin.header, &instructions).map(Some).map_err(|e| format!("write TITLE.BIN: {e:?}"))
}

/// Apply a box profile to an already-built scene (an exported translation): ADV spacing
/// only (the system script carries origin / height). A system script gets the full patch.
pub fn apply_box_profile_to_scene(bytes: &[u8], geometry: &box_profile::BoxGeometry, profile: &box_profile::BoxProfile, is_system: bool) -> Result<Vec<u8>, String> {
    let bin = parse_bin(bytes).map_err(|e| format!("parse: {e}"))?;
    let mut instructions = bin.instructions.clone();
    let changed = if is_system {
        box_profile::patch_system_script(&mut instructions, geometry, profile)
    } else {
        box_profile::patch_scene_script(&mut instructions, geometry, profile)
    };
    if changed == 0 {
        return Ok(bytes.to_vec());
    }
    write_bin(&bin.header, &instructions).map_err(|e| format!("write: {e:?}"))
}

/// True for the system script names a box profile patches in full.
pub fn is_system_script(name: &str) -> bool {
    SYSTEM_SCRIPTS.iter().any(|s| s.eq_ignore_ascii_case(name))
}

struct GameScripts {
    catalog_bytes: Vec<u8>,
    catalog: catalog::MasterCatalog,
    title: Vec<u8>,
    logo: Vec<u8>,
}

/// System scripts that define the text layouts, newest engine first.
const SYSTEM_SCRIPTS: [&str; 3] = ["SYSTEM4.BIN", "SYSTEM5.BIN", "SYSTEM3.BIN"];

/// The installed game folder holding `path` (a BIN inside the game or the game folder itself).
pub fn game_root_of(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|dir| master_path(dir).is_ok()).map(Path::to_path_buf)
}

/// Every text layout of a game (body box + all slots), plus the font a given script sets.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextLayout {
    pub body: box_profile::BoxGeometry,
    pub slots: Vec<box_profile::TextSlot>,
    /// First font size / spacing the tab's own BIN sets (SN, battle, UI scripts differ).
    pub script_font_px: Option<u32>,
    pub script_line_spacing: Option<u32>,
    /// Message-window art (band, name-plate frame, buttons) as the system script places it.
    pub art: Vec<box_art::BoxArtSprite>,
    /// The atlases `art` draws from, decoded to PNG data URLs.
    pub atlases: Vec<BoxAtlas>,
    /// Installed game folder (key for per-game settings such as the text box profile).
    pub game_dir: Option<String>,
    /// Body box of the BASE game files (no `.fanpatch`): what a text box profile applies to.
    pub base_body: Option<box_profile::BoxGeometry>,
    /// The engine's dialogue text metrics, when the loader has reported them
    /// (`fanpatch_layout.txt`, written by its word-wrap hook).
    pub metrics: Option<text_metrics::EngineTextMetrics>,
}

/// An AGF bound to a texture slot, as a data URL for the preview.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxAtlas {
    pub texture: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub data_url: String,
}

/// PNG data URL of a decoded AGF.
fn png_data_url(image: &AgfImage) -> Result<String, String> {
    use base64::Engine;
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let rgba: Vec<u8> = image.bgra.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&rgba))
            .map_err(|e| format!("png encode: {e}"))?;
    }
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png_bytes)))
}

/// Decode the atlases `art` uses (texture slot → catalog entry → AGF the game would load).
fn box_atlases(game_dir: &Path, system: &[crate::bin_format::instruction::Instruction], art: &[box_art::BoxArtSprite]) -> Vec<BoxAtlas> {
    let Ok(master_bytes) = master_path(game_dir).and_then(|p| std::fs::read(p).map_err(|e| e.to_string())) else { return Vec::new() };
    let Some(master) = alf::read_sys4_master(&master_bytes) else { return Vec::new() };
    let mut atlases = Vec::new();
    for (texture, catalog_index) in box_art::texture_bindings(system) {
        if !art.iter().any(|s| s.texture == texture) || atlases.iter().any(|a: &BoxAtlas| a.texture == texture) {
            continue;
        }
        let Some(entry) = master.entries.get(catalog_index as usize) else { continue };
        let Ok(bytes) = effective_file(game_dir, &master, &entry.name) else { continue };
        let Ok(image) = crate::agf::decode_agf(&bytes) else { continue };
        let Ok(data_url) = png_data_url(&image) else { continue };
        atlases.push(BoxAtlas { texture, name: entry.name.clone(), width: image.width, height: image.height, data_url });
    }
    atlases
}

/// The system script an installed game would load (fanpatch override → loose → archive).
fn system_script(game_dir: &Path) -> Result<Vec<u8>, String> {
    let master = alf::read_sys4_master(&std::fs::read(master_path(game_dir)?).map_err(|e| format!("read catalog: {e}"))?)
        .ok_or("failed to parse the master catalog")?;
    SYSTEM_SCRIPTS
        .iter()
        .find_map(|name| effective_file(game_dir, &master, name).ok())
        .ok_or_else(|| "no SYSTEM3/4/5.BIN in this game".to_string())
}

/// ADV body-box geometry of an installed game.
pub fn box_geometry(game_dir: &Path) -> Result<box_profile::BoxGeometry, String> {
    text_layout_from_system(&system_script(game_dir)?, None).map(|layout| layout.body)
}

/// Text layouts from system-script bytes; `script` = the tab's own instructions (font).
pub fn text_layout_from_system(
    system: &[u8],
    script: Option<&[crate::bin_format::instruction::Instruction]>,
) -> Result<TextLayout, String> {
    let bin = parse_bin(system).map_err(|e| format!("parse system script: {e}"))?;
    let body = box_profile::detect_geometry(&bin.instructions).ok_or("the system script defines no text layout slot 1")?;
    let font = script.and_then(box_profile::script_font);
    let body_bottom = (body.y + body.height) as i64;
    Ok(TextLayout {
        body,
        slots: box_profile::text_slots(&bin.instructions),
        script_font_px: font.map(|f| f.0),
        script_line_spacing: font.and_then(|f| f.1),
        art: box_art::box_art(&bin.instructions, body.y as i64, body_bottom),
        atlases: Vec::new(), // filled by the caller when the game folder is known
        game_dir: None,
        base_body: None,
        metrics: None,
    })
}

/// Talk-key speakers of an installed game (SYS4 438 scheme, see
/// `editor::characters::detect_talk_key_scheme`): talk key → character index, and
/// character index → name (CHINIT, the copy the game would load — translated names win).
#[derive(Debug, Default, Clone)]
pub struct TalkKeySpeakers {
    pub owners: std::collections::HashMap<u32, u32>,
    pub names: std::collections::HashMap<u32, String>,
}

/// Change stamp of the files speaker tables are read from: newest modification time among
/// the master catalog, `.fanpatch` files and loose `*INIT*.BIN` in the game folder.
pub fn game_files_stamp(game_dir: &Path) -> u128 {
    let modified = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    };
    let mut stamp = master_path(game_dir).map(|p| modified(&p)).unwrap_or(0);
    if let Ok(entries) = std::fs::read_dir(game_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_uppercase();
            if name.ends_with(".FANPATCH") || (name.contains("INIT") && name.ends_with(".BIN")) {
                stamp = stamp.max(modified(&entry.path()));
            }
        }
    }
    stamp
}

/// Speaker tables of one installed game, built once per change stamp.
#[derive(Debug, Default)]
pub struct GameSpeakerTables {
    pub name_scripts: Vec<(String, Vec<crate::bin_format::instruction::Instruction>)>,
    /// Talk-key resolution per scene table address (SYS4 438).
    pub talk_keys: std::sync::Mutex<std::collections::HashMap<u32, Option<TalkKeySpeakers>>>,
}

impl GameSpeakerTables {
    pub fn load(game_dir: &Path) -> Self {
        Self { name_scripts: name_table_scripts(game_dir).unwrap_or_default(), talk_keys: Default::default() }
    }

    pub fn talk_key_speakers(&self, game_dir: &Path, table: u32) -> Option<TalkKeySpeakers> {
        self.talk_keys
            .lock()
            .unwrap()
            .entry(table)
            .or_insert_with(|| talk_key_speakers(game_dir, table).ok())
            .clone()
    }
}

/// Name-table scripts (CNINIT, CHINIT; append copies before base) the game would load,
/// in `editor::characters::NAME_TABLE_FILES` priority.
pub fn name_table_scripts(game_dir: &Path) -> Result<Vec<(String, Vec<crate::bin_format::instruction::Instruction>)>, String> {
    let master = alf::read_sys4_master(&std::fs::read(master_path(game_dir)?).map_err(|e| format!("read catalog: {e}"))?)
        .ok_or("failed to parse the master catalog")?;
    let mut scripts = Vec::new();
    for file in crate::editor::characters::NAME_TABLE_FILES {
        let mut copies: Vec<&alf::AlfEntry> = master
            .entries
            .iter()
            .filter(|e| {
                let name = e.name.to_ascii_uppercase();
                name == file || (name.starts_with('$') && name.ends_with(&format!("${file}")))
            })
            .collect();
        copies.sort_by_key(|e| std::cmp::Reverse(e.name.starts_with('$')));
        for entry in copies {
            if let Ok(bin) = effective_file(game_dir, &master, &entry.name).and_then(|bytes| parse_bin(&bytes).map_err(|e| e.to_string())) {
                scripts.push((entry.name.clone(), bin.instructions));
            }
        }
    }
    Ok(scripts)
}

pub fn talk_key_speakers(game_dir: &Path, table: u32) -> Result<TalkKeySpeakers, String> {
    use crate::editor::characters::{extract_chinit_id_name_map, talk_key_owners, talk_key_source};
    let master = alf::read_sys4_master(&std::fs::read(master_path(game_dir)?).map_err(|e| format!("read catalog: {e}"))?)
        .ok_or("failed to parse the master catalog")?;
    let script = |name: &str| -> Option<Vec<crate::bin_format::instruction::Instruction>> {
        parse_bin(&effective_file(game_dir, &master, name).ok()?).ok().map(|b| b.instructions)
    };
    let init2 = script("INIT2.BIN").ok_or("INIT2.BIN not found")?;
    let (source, stride) = talk_key_source(&init2, table).ok_or("INIT2 does not fill the talk-key table")?;
    let mut result = TalkKeySpeakers::default();
    let init_names: Vec<String> = master
        .entries
        .iter()
        .map(|e| e.name.to_ascii_uppercase())
        .filter(|n| n.ends_with("INIT.BIN"))
        .collect();
    for name in &init_names {
        if let Some(instructions) = script(name) {
            talk_key_owners(&instructions, source, stride, &mut result.owners);
            if name.trim_start_matches(|c: char| c == '$' || c.is_ascii_digit()) == "CHINIT.BIN" {
                for (id, character) in extract_chinit_id_name_map(&instructions) {
                    result.names.entry(id).or_insert(character);
                }
            }
        }
    }
    Ok(result)
}

fn load_scripts(game_dir: &Path) -> Result<GameScripts, String> {
    let master_file = master_path(game_dir)?;
    let catalog_bytes = std::fs::read(&master_file).map_err(|e| format!("read {}: {e}", master_file.display()))?;
    let catalog = catalog::read_catalog(&catalog_bytes)?;
    let master = alf::read_sys4_master(&catalog_bytes).ok_or("catalog: index unreadable")?;
    let title = effective_file(game_dir, &master, "TITLE.BIN")?;
    let logo = effective_file(game_dir, &master, "LOGO.BIN")?;
    Ok(GameScripts { catalog_bytes, catalog, title, logo })
}

/// Inspect a game folder: identity, canvas, and which branding is possible.
pub fn inspect_branding(game_dir: &Path) -> Result<BrandingProfile, String> {
    let scripts = load_scripts(game_dir)?;
    let title = parse_bin(&scripts.title).map_err(|e| format!("TITLE.BIN: {e}"))?;
    let logo = parse_bin(&scripts.logo).map_err(|e| format!("LOGO.BIN: {e}"))?;
    let (screen_width, screen_height) = scripts.catalog.screen_size();
    Ok(BrandingProfile {
        title: scripts.catalog.title.clone(),
        build: format!("SYS{} {}", scripts.catalog.generation(), scripts.catalog.build),
        screen_width,
        screen_height,
        badge: branding::plan_badge(&title.instructions),
        splash: branding::plan_splash(&logo.instructions),
    })
}

/// Art for the branding records (straight BGRA, top-down).
pub struct BrandingArt {
    pub badge: Option<AgfImage>,
    pub splash: Option<AgfImage>,
}

fn rewrite(bytes: &[u8], patch: impl FnOnce(&[crate::bin_format::instruction::Instruction]) -> Result<Vec<crate::bin_format::instruction::Instruction>, String>, name: &str) -> Result<Vec<u8>, String> {
    let bin = parse_bin(bytes).map_err(|e| format!("{name}: {e}"))?;
    let patched = patch(&bin.instructions)?;
    let written = write_bin(&bin.header, &patched).map_err(|e| format!("{name}: write failed: {e}"))?;
    let reparsed = parse_bin(&written).map_err(|e| format!("{name}: patched script does not re-parse: {e}"))?;
    if reparsed.instructions.len() != patched.len() {
        return Err(format!("{name}: patched script lost instructions"));
    }
    Ok(written)
}

/// Build the branding files (catalog + patched scripts + AGFs).
pub fn build_branding(game_dir: &Path, art: &BrandingArt) -> Result<Vec<(String, Vec<u8>)>, String> {
    if art.badge.is_none() && art.splash.is_none() {
        return Err("nothing to brand: provide badge and/or splash art".into());
    }
    let scripts = load_scripts(game_dir)?;
    let badge_agf = art.badge.as_ref().map(encode_agf);
    let splash_agf = art.splash.as_ref().map(encode_agf);
    let mut records = Vec::new();
    if let Some(agf) = &badge_agf {
        records.push(catalog::NewRecord { name: BADGE_RECORD, length: agf.len() as u32 });
    }
    if let Some(agf) = &splash_agf {
        records.push(catalog::NewRecord { name: SPLASH_RECORD, length: agf.len() as u32 });
    }
    let (catalog_bytes, first_id) = catalog::extend_catalog(&scripts.catalog_bytes, &records)?;
    let mut next_id = first_id;
    let master_name = master_path(game_dir)?.file_name().unwrap().to_string_lossy().into_owned();
    let mut files = vec![(master_name, catalog_bytes)];

    if let Some(agf) = badge_agf {
        let badge_id = next_id;
        next_id += 1;
        let title = rewrite(&scripts.title, |ins| branding::apply_badge(ins, &branding::plan_badge(ins), badge_id), "TITLE.BIN")?;
        files.push(("TITLE.BIN".into(), title));
        files.push((BADGE_RECORD.into(), agf));
    }
    if let Some(agf) = splash_agf {
        let splash_id = next_id;
        let logo = rewrite(&scripts.logo, |ins| branding::apply_splash(ins, &branding::plan_splash(ins), splash_id), "LOGO.BIN")?;
        files.push(("LOGO.BIN".into(), logo));
        files.push((SPLASH_RECORD.into(), agf));
    }
    Ok(files)
}

/// Build and write `zz_branding.fanpatch` into the game folder (or `output`).
pub fn write_branding_fanpatch(game_dir: &Path, art: &BrandingArt, output: Option<&Path>) -> Result<PathBuf, String> {
    let files = build_branding(game_dir, art)?;
    let target = output.map(Path::to_path_buf).unwrap_or_else(|| game_dir.join(BRANDING_FANPATCH));
    let file = std::fs::File::create(&target).map_err(|e| format!("create {}: {e}", target.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in &files {
        zip.start_file(name.as_str(), options).map_err(|e| format!("zip {name}: {e}"))?;
        zip.write_all(bytes).map_err(|e| format!("zip {name}: {e}"))?;
    }
    zip.finish().map_err(|e| format!("zip finish: {e}"))?;
    Ok(target)
}

/// Decode a `data:image/png;base64,…` URL (as produced by a canvas) into BGRA.
pub fn image_from_png_data_url(data_url: &str) -> Result<AgfImage, String> {
    use base64::Engine;
    let encoded = data_url.split_once("base64,").map(|(_, b)| b).unwrap_or(data_url);
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).map_err(|e| format!("art: bad base64: {e}"))?;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("art: bad PNG: {e}"))?;
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buffer).map_err(|e| format!("art: PNG decode: {e}"))?;
    let (width, height) = (frame.width, frame.height);
    let pixels = &buffer[..frame.buffer_size()];
    let channels = match frame.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        other => return Err(format!("art: unsupported PNG colour type {other:?}")),
    };
    let mut bgra = Vec::with_capacity((width * height * 4) as usize);
    for px in pixels.chunks_exact(channels) {
        let (r, g, b, a) = match channels {
            4 => (px[0], px[1], px[2], px[3]),
            3 => (px[0], px[1], px[2], 255),
            2 => (px[0], px[0], px[0], px[1]),
            _ => (px[0], px[0], px[0], 255),
        };
        bgra.extend_from_slice(&[b, g, r, a]);
    }
    Ok(AgfImage { width, height, bgra })
}

/// Text layouts for a tab's game (game-accurate dialog preview). Looks in the
/// installed game holding the tab's BIN, then `game_dir_hint` (workspace tabs are
/// copies outside the game), then a system script beside the BIN.
#[tauri::command]
pub async fn engine_text_layout(
    tab_id: String,
    game_dir_hint: Option<String>,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<TextLayout, String> {
    let (bin_path, script) = {
        let tabs = state.tabs.lock().unwrap();
        let tab = tabs.get(&tab_id);
        (tab.and_then(|t| t.bin_path.clone()), tab.and_then(|t| t.parsed_bin.as_ref()).map(|b| b.instructions.clone()))
    };
    let bin_path = bin_path.map(PathBuf::from);
    let root = bin_path
        .as_deref()
        .and_then(game_root_of)
        .or_else(|| game_dir_hint.filter(|h| !h.is_empty()).and_then(|h| game_root_of(Path::new(&h))));
    let system = match &root {
        Some(root) => system_script(root)?,
        None => {
            let beside = bin_path
                .as_deref()
                .and_then(Path::parent)
                .and_then(|dir| SYSTEM_SCRIPTS.iter().map(|n| dir.join(n)).find(|p| p.exists()))
                .ok_or("game folder not found for this tab")?;
            std::fs::read(&beside).map_err(|e| format!("read {}: {e}", beside.display()))?
        }
    };
    let mut layout = text_layout_from_system(&system, script.as_deref())?;
    if let Some(root) = &root {
        let instructions = parse_bin(&system).map(|b| b.instructions).unwrap_or_default();
        layout.atlases = box_atlases(root, &instructions, &layout.art);
        layout.game_dir = Some(root.display().to_string());
        layout.base_body = base_box_geometry(root).ok();
        layout.metrics = text_metrics::read(root);
    }
    Ok(layout)
}

/// Which branding the game supports and at what sizes (UI renders art to fit).
#[tauri::command]
pub async fn engine_inspect_branding(game_dir: String) -> Result<BrandingProfile, String> {
    inspect_branding(Path::new(&game_dir))
}

/// Write `zz_branding.fanpatch` into the game folder from canvas-rendered PNGs.
#[tauri::command]
pub async fn engine_build_branding(game_dir: String, badge_png: Option<String>, splash_png: Option<String>) -> Result<String, String> {
    let art = BrandingArt {
        badge: badge_png.as_deref().map(image_from_png_data_url).transpose()?,
        splash: splash_png.as_deref().map(image_from_png_data_url).transpose()?,
    };
    write_branding_fanpatch(Path::new(&game_dir), &art, None).map(|p| p.to_string_lossy().into_owned())
}

/// Read a user-picked image (splash art) as a data URL for the canvas renderer.
#[tauri::command]
pub async fn engine_read_image_data_url(path: String) -> Result<String, String> {
    use base64::Engine;
    let bytes = std::fs::read(&path).map_err(|e| format!("read {path}: {e}"))?;
    let lower = path.to_ascii_lowercase();
    let mime = if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".bmp") {
        "image/bmp"
    } else {
        "image/jpeg"
    };
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}
