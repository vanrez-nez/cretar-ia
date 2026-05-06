use crate::commands::settings;
use crate::runtime::control;
use crate::settings_control;
use anyhow::{Result, anyhow};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use tauri::{Theme, utils::config::Color};

const SETTINGS_BACKGROUND: Color = Color(17, 19, 22, 255);

pub fn run() -> Result<()> {
    let state = settings::build_settings_state().map_err(anyhow::Error::msg)?;

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            settings::load_config,
            settings::save_config,
            settings::get_config_path,
            settings::open_config_file,
            settings::get_settings,
            settings::update_settings,
            settings::check_permissions,
            settings::request_microphone_permission,
            settings::request_accessibility_permission,
        ])
        .setup(|app| {
            let _window = tauri::WebviewWindowBuilder::new(
                app,
                "settings",
                settings_url(),
            )
            .title("Cretar IA Settings")
            .inner_size(880.0, 680.0)
            .resizable(true)
            .theme(Some(Theme::Dark))
            .background_color(SETTINGS_BACKGROUND)
            .build()
            .map_err(|err| anyhow!(err.to_string()))?;
            let _settings_control = settings_control::spawn_settings_control_server(app.handle().clone())
                .map_err(|err| anyhow!(err.to_string()))?;
            if let Err(err) = control::notify_hotkeys_pause_for_settings() {
                log::debug!("runtime hotkey pause notification skipped: {err}");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "settings" && matches!(event, tauri::WindowEvent::Destroyed) {
                if let Err(err) = control::notify_hotkeys_resume_after_settings() {
                    log::debug!("runtime hotkey resume notification skipped: {err}");
                }
                settings_control::cleanup_settings_control_endpoint();
            }
        })
        .run(tauri::generate_context!())
        .map_err(|err| anyhow!(err.to_string()))?;

    Ok(())
}

fn settings_url() -> tauri::WebviewUrl {
    #[cfg(debug_assertions)]
    {
        if dev_server_is_available() {
            if let Ok(url) = "http://localhost:5173".parse() {
                return tauri::WebviewUrl::External(url);
            }
        }
    }

    tauri::WebviewUrl::App("index.html".into())
}

#[cfg(debug_assertions)]
fn dev_server_is_available() -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], 5173));
    TcpStream::connect_timeout(&addr, Duration::from_millis(150)).is_ok()
}
