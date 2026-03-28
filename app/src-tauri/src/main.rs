// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio_commands;
mod commands;
mod datacore_commands;
mod state;
mod ui_sink;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(state::AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::discover_p4k,
            commands::open_p4k,
            commands::list_dir,
            commands::list_subdirs,
            commands::scan_categories,
            commands::start_export,
            commands::cancel_export,
            datacore_commands::dc_search,
            datacore_commands::dc_list_tree,
            datacore_commands::dc_get_record,
            datacore_commands::dc_export_json,
            datacore_commands::dc_export_xml,
            datacore_commands::dc_get_backlinks,
            audio_commands::audio_init,
            audio_commands::audio_search_entities,
            audio_commands::audio_search_triggers,
            audio_commands::audio_list_banks,
            audio_commands::audio_bank_triggers,
            audio_commands::audio_bank_media,
            audio_commands::audio_entity_triggers,
            audio_commands::audio_resolve_trigger,
            audio_commands::audio_decode_wem,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
