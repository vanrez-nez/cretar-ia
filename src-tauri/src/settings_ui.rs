use crate::commands::settings;
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
            Ok(())
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
