// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio_commands;
mod commands;
mod datacore_commands;
mod error;
mod state;
mod ui_sink;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(state::AppState::new())
        .setup(|app| {
            // Watch for OS theme changes and emit events to the frontend.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let Ok(st) = system_theme::SystemTheme::new() else {
                    return;
                };
                let stream = st.subscribe();
                futures_lite::pin!(stream);
                use futures_lite::StreamExt;
                while stream.next().await.is_some() {
                    let palette = commands::get_system_theme();
                    let _ = tauri::Emitter::emit(&handle, "system-theme-changed", &palette);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_system_theme,
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
            datacore_commands::dc_export_folder,
            audio_commands::audio_init,
            audio_commands::audio_search_entities,
            audio_commands::audio_search_triggers,
            audio_commands::audio_list_banks,
            audio_commands::audio_bank_triggers,
            audio_commands::audio_bank_media,
            audio_commands::audio_entity_triggers,
            audio_commands::audio_resolve_trigger,
            audio_commands::audio_decode_wem,
            commands::preview_geometry,
            commands::preview_xml,
            commands::preview_dds,
            commands::read_p4k_file,
            commands::extract_p4k_folder,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            eprintln!("fatal: {e}");
            std::process::exit(1);
        });
}
