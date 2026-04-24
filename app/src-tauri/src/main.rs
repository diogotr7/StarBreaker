// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio_commands;
mod commands;
mod datacore_commands;
mod error;
mod state;
mod ui_sink;

#[cfg(target_os = "linux")]
use image::GenericImageView;
use tauri::Manager;

#[cfg(target_os = "linux")]
const LINUX_DESKTOP_FILE_ID: &str = "app.starbreaker";
#[cfg(target_os = "linux")]
const LINUX_ICON_NAME: &str = "app.starbreaker";
#[cfg(target_os = "linux")]
const LINUX_STARTUP_WM_CLASS: &str = "starbreaker-app";
#[cfg(target_os = "linux")]
const LINUX_ICON_BYTES: &[u8] = include_bytes!("../icons/icon.png");

#[cfg(target_os = "linux")]
fn ensure_linux_desktop_integration() -> std::io::Result<()> {
    if std::env::var_os("APPIMAGE").is_some() {
        return Ok(());
    }

    let Some(data_home) = linux_data_home() else {
        return Ok(());
    };

    let executable = std::env::current_exe()?;
    let (icon_width, icon_height) = image::load_from_memory_with_format(
        LINUX_ICON_BYTES,
        image::ImageFormat::Png,
    )
    .map(|icon| icon.dimensions())
    .unwrap_or((256, 256));

    let applications_dir = data_home.join("applications");
    let icons_dir = data_home
        .join("icons")
        .join("hicolor")
        .join(format!("{icon_width}x{icon_height}"))
        .join("apps");
    let primary_icon_path = icons_dir.join(format!("{LINUX_ICON_NAME}.png"));

    std::fs::create_dir_all(&applications_dir)?;
    std::fs::create_dir_all(&icons_dir)?;

    write_if_changed(&primary_icon_path, LINUX_ICON_BYTES)?;
    write_if_changed(
        &icons_dir.join(format!("{LINUX_STARTUP_WM_CLASS}.png")),
        LINUX_ICON_BYTES,
    )?;

    let desktop_entry = format!(
        concat!(
            "[Desktop Entry]\n",
            "Type=Application\n",
            "Name=StarBreaker\n",
            "Exec={}\n",
            "Icon={}\n",
            "StartupWMClass={}\n",
            "X-GNOME-WMClass={}\n",
            "StartupNotify=true\n",
            "Terminal=false\n",
            "Categories=Graphics;Utility;\n"
        ),
        desktop_exec_path(&executable),
        desktop_icon_path(&primary_icon_path),
        LINUX_STARTUP_WM_CLASS,
        LINUX_STARTUP_WM_CLASS,
    );

    for desktop_file_id in [LINUX_DESKTOP_FILE_ID, LINUX_STARTUP_WM_CLASS] {
        write_if_changed(
            &applications_dir.join(format!("{desktop_file_id}.desktop")),
            desktop_entry.as_bytes(),
        )?;
    }

    refresh_linux_desktop_caches(&applications_dir, &data_home.join("icons")).ok();

    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_data_home() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|home| home.join(".local").join("share"))
        })
}

#[cfg(target_os = "linux")]
fn desktop_exec_path(path: &std::path::Path) -> String {
    let mut escaped = String::from("\"");
    for ch in path.to_string_lossy().chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

#[cfg(target_os = "linux")]
fn desktop_icon_path(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(target_os = "linux")]
fn write_if_changed(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    match std::fs::read(path) {
        Ok(existing) if existing == contents => Ok(()),
        Ok(_) | Err(_) => std::fs::write(path, contents),
    }
}

#[cfg(target_os = "linux")]
fn refresh_linux_desktop_caches(
    applications_dir: &std::path::Path,
    icons_root: &std::path::Path,
) -> std::io::Result<()> {
    run_linux_command_if_available(
        "update-desktop-database",
        [applications_dir.as_os_str()],
    )?;
    run_linux_command_if_available(
        "gtk-update-icon-cache",
        [std::ffi::OsStr::new("-f"), std::ffi::OsStr::new("-t"), icons_root.as_os_str()],
    )?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn run_linux_command_if_available<I, S>(command: &str, args: I) -> std::io::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    use std::process::Command;

    let status = match Command::new(command).args(args).status() {
        Ok(status) => status,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "{command} exited with status {status}"
        )))
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .target(tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview))
                .target(tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout))
                .target(tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir { file_name: None }))
                .build(),
        )
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(state::AppState::new())
        .setup(|app| {
            #[cfg(target_os = "linux")]
            if let Err(error) = ensure_linux_desktop_integration() {
                log::warn!(
                    "failed to install desktop metadata for direct Linux binary launches: {error}"
                );
            }

            if let Some(icon) = app.default_window_icon().cloned() {
                for window in app.webview_windows().values() {
                    let _ = window.set_icon(icon.clone());
                }
            }

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
            commands::export_dds_png,
            commands::extract_p4k_file,
            commands::read_p4k_file,
            commands::extract_p4k_folder,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            eprintln!("fatal: {e}");
            std::process::exit(1);
        });
}
