use crate::config::AppConfig;
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::fs;
use tauri::State;

struct SettingsState {
    config_path: String,
}

#[tauri::command]
fn load_config(state: State<'_, SettingsState>) -> Result<Value, String> {
    let raw = fs::read_to_string(&state.config_path).map_err(|err| err.to_string())?;
    serde_json::from_str::<Value>(&raw).map_err(|err| err.to_string())
}

#[tauri::command]
fn save_config(config: Value, state: State<'_, SettingsState>) -> Result<(), String> {
    let payload = serde_json::to_string_pretty(&config).map_err(|err| err.to_string())?;
    fs::write(&state.config_path, payload).map_err(|err| err.to_string())?;
    Ok(())
}

#[tauri::command]
fn get_config_path(state: State<'_, SettingsState>) -> Result<String, String> {
    Ok(state.config_path.clone())
}

#[tauri::command]
fn open_config_file(state: State<'_, SettingsState>) -> Result<(), String> {
    tauri_plugin_opener::open_path(&state.config_path, None::<&str>)
        .map_err(|err| err.to_string())
}

pub fn run() -> Result<()> {
    let config_path = AppConfig::config_path();
    AppConfig::load_or_create().with_context(|| {
        format!("preparing config at {}", config_path.display())
    })?;

    let state = SettingsState {
        config_path: config_path.to_string_lossy().into_owned(),
    };

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            get_config_path,
            open_config_file
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
