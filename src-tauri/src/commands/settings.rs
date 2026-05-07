use crate::config::AppConfig;
use crate::permissions::PermissionsStatus;
use crate::settings_db::SettingsDb;
use serde_json::Value;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn load_config(storage: State<'_, SettingsDb>) -> Result<AppConfig, String> {
    storage.load_config().await.map_err(command_error)
}

#[tauri::command]
pub async fn save_config(
    config: Value,
    save_id: Option<u64>,
    app: AppHandle,
    storage: State<'_, SettingsDb>,
) -> Result<AppConfig, String> {
    let previous = storage.load_config().await.ok();
    log::info!(
        "settings save received save_id={:?} fingerprint={}",
        save_id,
        config_value_fingerprint(&config)
    );
    let config = serde_json::from_value::<AppConfig>(config)
        .map_err(|err| format!("invalid config schema: {err}"))?;
    let config = storage.save_config(config).await.map_err(command_error)?;
    log::info!(
        "settings save persisted save_id={:?} fingerprint={}",
        save_id,
        config_fingerprint(&config)
    );
    apply_saved_config(&app, previous.as_ref(), &config, save_id)?;
    Ok(config)
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
pub async fn get_settings(storage: State<'_, SettingsDb>) -> Result<AppConfig, String> {
    storage.load_config().await.map_err(command_error)
}

#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    storage: State<'_, SettingsDb>,
    config: AppConfig,
) -> Result<(), String> {
    let previous = storage.load_config().await.ok();
    log::info!(
        "settings typed update received fingerprint={}",
        config_fingerprint(&config)
    );
    let config = storage.save_config(config).await.map_err(command_error)?;
    log::info!(
        "settings typed update persisted fingerprint={}",
        config_fingerprint(&config)
    );
    apply_saved_config(&app, previous.as_ref(), &config, None)?;
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

fn apply_saved_config(
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

fn config_value_fingerprint(config: &Value) -> String {
    serde_json::json!({
        "language": config.pointer("/ui/language"),
        "mode": config.pointer("/interaction/mode"),
        "shortcut": config.pointer("/interaction/shortcut"),
        "input_device": config.pointer("/audio/input_device"),
        "auto_switch_input": config.pointer("/audio/auto_switch_to_primary_device"),
        "start_sound": config.pointer("/audio_cues/start_sound"),
        "stop_sound": config.pointer("/audio_cues/stop_sound"),
        "error_sound": config.pointer("/audio_cues/error_sound"),
    })
    .to_string()
}

fn config_fingerprint(config: &AppConfig) -> String {
    serde_json::json!({
        "language": config.ui.language,
        "mode": config.interaction.mode,
        "shortcut": config.interaction.shortcut,
        "input_device": config.audio.input_device,
        "auto_switch_input": config.audio.auto_switch_to_primary_device,
        "start_sound": config.audio_cues.start_sound,
        "stop_sound": config.audio_cues.stop_sound,
        "error_sound": config.audio_cues.error_sound,
    })
    .to_string()
}
