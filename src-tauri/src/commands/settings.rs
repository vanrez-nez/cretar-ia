use crate::config::AppConfig;
use crate::permissions::PermissionsStatus;
use crate::settings_db::SettingsDb;
use serde_json::Value;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn apply_settings(
    save_id: Option<u64>,
    app: AppHandle,
    storage: State<'_, SettingsDb>,
) -> Result<(), String> {
    let previous = crate::app_host::current_config(&app);
    let settings = storage.load_settings().await.map_err(command_error)?;
    let config = crate::settings_schema::runtime_config_from_settings(&settings).map_err(command_error)?;

    log::info!(
        "settings apply requested save_id={:?} fingerprint={}",
        save_id,
        settings_fingerprint(&settings)
    );
    apply_saved_config(&app, previous.as_ref(), &config, save_id)
}

#[tauri::command]
pub async fn get_config_path(storage: State<'_, SettingsDb>) -> Result<String, String> {
    Ok(storage.db_path().to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn open_config_file(storage: State<'_, SettingsDb>) -> Result<(), String> {
    tauri_plugin_opener::open_path(storage.db_path(), None::<&str>)
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn list_input_devices() -> Result<Vec<String>, String> {
    Ok(crate::audio::available_input_device_names())
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

pub(crate) fn apply_saved_config(
    app: &AppHandle,
    previous: Option<&AppConfig>,
    config: &AppConfig,
    save_id: Option<u64>,
) -> Result<(), String> {
    log::info!(
        "settings tray refresh started save_id={:?} fingerprint={}",
        save_id,
        config_fingerprint(config)
    );
    crate::app_host::refresh_tray_menu(app, config);
    log::info!("settings tray refresh finished save_id={:?}", save_id);

    let should_restart = requires_runtime_restart(previous, config);
    log::info!(
        "settings runtime apply decision save_id={:?} restart={} fingerprint={}",
        save_id,
        should_restart,
        config_fingerprint(config)
    );

    if should_restart {
        log::info!("settings runtime apply started save_id={:?}", save_id);
        crate::app_host::restart_runtime(app, config.clone())
            .map_err(|err| err.to_string())?;
        log::info!("settings runtime apply finished save_id={:?}", save_id);
    }

    Ok(())
}

fn command_error(err: anyhow::Error) -> String {
    err.to_string()
}

fn requires_runtime_restart(previous: Option<&AppConfig>, next: &AppConfig) -> bool {
    let Some(previous) = previous else {
        return true;
    };

    runtime_config_value(previous) != runtime_config_value(next)
}

fn runtime_config_value(config: &AppConfig) -> Value {
    serde_json::json!({
        "provider": &config.provider,
        "interaction": &config.interaction,
        "pipeline": &config.pipeline,
        "audio": &config.audio,
        "audio_cues": &config.audio_cues,
        "output": &config.output,
    })
}

fn settings_fingerprint(settings: &Value) -> String {
    serde_json::json!({
        "system.language": settings.get("system.language"),
        "recording.mode": settings.get("recording.mode"),
        "recording.hotkey": settings.get("recording.hotkey"),
        "recording.microphone.input_device": settings.get("recording.microphone.input_device"),
        "recording.sounds.start": settings.get("recording.sounds.start"),
        "recording.sounds.stop": settings.get("recording.sounds.stop"),
        "recording.sounds.error": settings.get("recording.sounds.error"),
    })
    .to_string()
}

fn config_fingerprint(config: &AppConfig) -> String {
    serde_json::json!({
        "system.language": config.ui.language,
        "recording.mode": config.interaction.mode,
        "recording.hotkey": config.interaction.shortcut,
        "recording.microphone.input_device": config.audio.input_device,
        "recording.sounds.start": config.audio_cues.start_sound,
        "recording.sounds.stop": config.audio_cues.stop_sound,
        "recording.sounds.error": config.audio_cues.error_sound,
    })
    .to_string()
}
