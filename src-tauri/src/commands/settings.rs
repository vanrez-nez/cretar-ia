use crate::config::AppConfig;
use anyhow::Context;
use serde_json::Value;
use tauri::State;

#[derive(Clone)]
pub struct SettingsState {
    pub config_path: String,
}

#[tauri::command]
pub async fn load_config(state: State<'_, SettingsState>) -> Result<AppConfig, String> {
    let raw = std::fs::read_to_string(&state.config_path).map_err(|err| err.to_string())?;
    AppConfig::parse(&raw).map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn save_config(
    config: Value,
    state: State<'_, SettingsState>,
) -> Result<(), String> {
    let config = serde_json::from_value::<AppConfig>(config).map_err(|err| err.to_string())?;
    config.validate().map_err(|err| err.to_string())?;
    let payload = serde_json::to_string_pretty(&config).map_err(|err| err.to_string())?;
    std::fs::write(&state.config_path, payload).map_err(|err| err.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_config_path(state: State<'_, SettingsState>) -> Result<String, String> {
    Ok(state.config_path.clone())
}

#[tauri::command]
pub async fn open_config_file(state: State<'_, SettingsState>) -> Result<(), String> {
    tauri_plugin_opener::open_path(&state.config_path, None::<&str>)
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn get_settings(state: State<'_, SettingsState>) -> Result<AppConfig, String> {
    load_config(state).await
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, SettingsState>,
    config: AppConfig,
) -> Result<(), String> {
    config.validate().map_err(|err| err.to_string())?;
    let payload = serde_json::to_string_pretty(&config).map_err(|err| err.to_string())?;
    std::fs::write(&state.config_path, payload).map_err(|err| err.to_string())?;
    Ok(())
}

pub fn build_settings_state() -> Result<SettingsState, String> {
    let path = AppConfig::config_path();
    AppConfig::load_or_create().with_context(|| {
        format!("preparing config at {}", path.display())
    })
    .map_err(|err| err.to_string())?;

    Ok(SettingsState {
        config_path: path.to_string_lossy().into_owned(),
    })
}
