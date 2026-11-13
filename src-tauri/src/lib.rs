pub mod agf;
pub mod alf;
pub mod assets;
pub mod bin_format;
pub mod claude;
pub mod commands;
pub mod editor;
pub mod engine;
pub mod parente;
pub mod project;
pub mod state;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = AppState::new();

    // Try to load API key from OS keyring on startup
    if let Ok(entry) = keyring::Entry::new("eushully-bin-editor", "claude-api-key") {
        if let Ok(key) = entry.get_password() {
            *app_state.api_key.lock().unwrap() = Some(key);
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(app_state)
        .manage(assets::AssetsState::default())
        .register_asynchronous_uri_scheme_protocol(assets::protocol::SCHEME, assets::protocol::handle_request)
        .invoke_handler(tauri::generate_handler![
            commands::load_bin,
            commands::get_engine_info,
            commands::load_project,
            commands::load_bin_for_project,
            commands::save_project,
            commands::export_bin,
            commands::export_fanpatch,
            commands::pack_fanpatch_from_dir,
            commands::pack_dual_language_fanpatch,
            commands::set_api_key,
            commands::get_api_key,
            commands::set_provider_config,
            commands::get_provider_config,
            commands::translate_single_entry,
            commands::optimize_entry,
            commands::proofread_block,
            commands::start_translation,
            commands::pause_translation,
            commands::resume_translation,
            commands::stop_translation,
            commands::get_referenced_speakers,
            commands::get_row_speakers,
            commands::scan_bin_for_characters,
            commands::get_all_instructions,
            commands::compute_file_hash,
            commands::close_tab,
            commands::get_bridge_template_status,
            commands::get_split_capability,
            commands::auto_split_entry,
            commands::auto_split_entries_batch,
            commands::export_glossary,
            commands::import_glossary,
            commands::fetch_available_models,
            commands::fetch_openai_compat_models,
            commands::scan_cross_bin_context,
            commands::load_settings,
            commands::save_settings,
            commands::file_exists,
            commands::ensure_directory,
            commands::export_for_translation,
            commands::import_translations,
            commands::get_bridge_clone,
            commands::get_semantic_bridge,
            commands::export_glossary_cli,
            commands::read_glossary_cli_results,
            commands::lookup_character_names_for_pointers,
            commands::list_init_bin_names,
            commands::open_folder_in_explorer,
            commands::scan_bin_folder,
            commands::extract_alf_to_folder,
            commands::path_is_dir,
            commands::is_folder_writable,
            commands::copy_files_to_folder,
            commands::relaunch_translation_folder,
            commands::list_project_backups,
            commands::diff_project_backup,
            commands::restore_project_backup,
            commands::read_prior_translation_map,
            commands::load_folder_glossary,
            commands::save_folder_glossary,
            commands::scan_translation_project_status,
            commands::write_multi_translate_launchers,
            parente::parente_list_arc,
            parente::parente_load_entry,
            parente::parente_export_arc,
            parente::parente_list_game,
            parente::parente_get_game_language,
            parente::parente_set_game_language,
            parente::parente_upgrade_patch,
            parente::parente_apply_font_fix,
            parente::parente_revert_font_fix,
            engine::engine_inspect_branding,
            engine::engine_text_layout,
            commands::resolve_speaker_names,
            engine::engine_build_branding,
            engine::engine_read_image_data_url,
            parente::parente_load_game_entry,
            parente::parente_export_patch,
            parente::parente_export_full,
            parente::parente_export_english,
            parente::parente_suggested_patch_path,
            parente::parente_save_project,
            parente::parente_load_project,
            assets::commands::assets_startup_request,
            assets::commands::assets_find_entry,
            assets::commands::assets_open_game,
            assets::commands::assets_close_game,
            assets::commands::assets_list,
            assets::commands::assets_list_indices,
            assets::commands::assets_preview,
            assets::commands::assets_extract,
            assets::commands::assets_cancel_extract,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod test_show_text_decode {
    use crate::bin_format::parser::parse_bin;
    use std::path::Path;
    
    fn load_test_bin(name: &str) -> Option<Vec<u8>> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(name);
        std::fs::read(&path).ok()
    }

    #[test]
    fn test_print_decoded_show_text() {
        println!("\n=== SC1310.BIN - Decoded show-text Instructions ===\n");
        let Some(data) = load_test_bin("SC1310.BIN") else { return };
        let parsed = parse_bin(&data).expect("Failed to parse SC1310.BIN");
        
        let show_text_instrs: Vec<_> = parsed.instructions
            .iter()
            .filter(|i| i.opcode == 0x6E)
            .collect();
        
        for (idx, instr) in show_text_instrs.iter().take(10).enumerate() {
            println!("[show-text #{}]", idx + 1);
            for (arg_idx, arg) in instr.arguments.iter().enumerate() {
                println!("  Arg {}: type={}", arg_idx, arg.arg_type);
                if let Some(ref s) = arg.decoded_string {
                    println!("    STRING: '{}'", s);
                } else {
                    println!("    VALUE: 0x{:X}", arg.raw_data);
                }
            }
            println!();
        }
    }
}
