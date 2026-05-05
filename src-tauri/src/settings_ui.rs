use crate::commands::settings;
use anyhow::{Result, anyhow};

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
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Cretar IA Settings")
            .inner_size(880.0, 680.0)
            .resizable(true)
            .build()
            .map_err(|err| anyhow!(err.to_string()))?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .map_err(|err| anyhow!(err.to_string()))?;

    Ok(())
}
