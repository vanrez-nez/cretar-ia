use crate::commands::settings_service::SettingsService;
use crate::config::AppConfig;
use crate::permissions::PermissionsStatus;
use serde_json::Value;
use tauri::{AppHandle, State};

#[derive(Clone)]
pub struct SettingsState {
    pub config_path: String,
}

#[tauri::command]
pub async fn load_config(state: State<'_, SettingsState>) -> Result<AppConfig, String> {
    state
        .service()
        .load()
        .map_err(SettingsService::command_error)
}

#[tauri::command]
pub async fn save_config(
    config: Value,
    app: AppHandle,
    state: State<'_, SettingsState>,
) -> Result<(), String> {
    let config = state
        .service()
        .save_value(config)
        .map_err(SettingsService::command_error)?;
    crate::app_host::restart_runtime(&app, config)
        .map_err(|err| err.to_string())?;
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
    state
        .service()
        .load()
        .map_err(SettingsService::command_error)
}

#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    state: State<'_, SettingsState>,
    config: AppConfig,
) -> Result<(), String> {
    state
        .service()
        .save(config.clone())
        .map_err(SettingsService::command_error)?;
    crate::app_host::restart_runtime(&app, config)
        .map_err(|err| err.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn check_permissions() -> Result<PermissionsStatus, String> {
    Ok(crate::permissions::check_permissions().await)
}

#[tauri::command]
pub async fn request_microphone_permission() -> Result<crate::permissions::PermissionState, String> {
    Ok(crate::permissions::request_microphone_permission().await)
}

#[tauri::command]
pub async fn request_accessibility_permission() -> Result<crate::permissions::PermissionState, String> {
    Ok(crate::permissions::request_accessibility_permission().await)
}

pub fn build_settings_state() -> Result<SettingsState, String> {
    let service = SettingsService::prepare_default().map_err(SettingsService::command_error)?;

    Ok(SettingsState {
        config_path: service.config_path().to_string_lossy().into_owned(),
    })
}

impl SettingsState {
    fn service(&self) -> SettingsService {
        SettingsService::new(&self.config_path)
    }
}
