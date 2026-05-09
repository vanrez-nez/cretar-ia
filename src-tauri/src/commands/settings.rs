use crate::config::AppConfig;
use crate::model_health::{ModelHealthCache, ModelHealthView};
use crate::permissions::PermissionsStatus;
use crate::prompts::{PromptCache, PromptView};
use crate::providers::{ProviderModelOption, RoleModelSettings};
use crate::settings_db::SettingsDb;
use anyhow::{anyhow, Context};
use rodio::Source;
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;
use zip::write::SimpleFileOptions;

static HISTORY_EXPORTS: LazyLock<Mutex<HashMap<String, PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryExportStart {
    export_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryExportSaveResult {
    path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryExportProgress {
    export_id: String,
    status: &'static str,
    processed: usize,
    total: usize,
    message: Option<String>,
}

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
pub async fn list_sound_options() -> Result<Vec<crate::config::SoundOption>, String> {
    crate::config::sound_options().map_err(command_error)
}

#[tauri::command]
pub async fn get_history_overview(
    storage: State<'_, SettingsDb>,
) -> Result<crate::history::HistoryOverview, String> {
    crate::history::overview(&storage.pool())
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn get_latest_history_audio(
    storage: State<'_, SettingsDb>,
) -> Result<Option<crate::history::HistoryAudioItem>, String> {
    crate::history::latest_audio(&storage.pool())
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn list_history_records(
    page: Option<u32>,
    page_size: Option<u32>,
    storage: State<'_, SettingsDb>,
) -> Result<crate::history::HistoryPage, String> {
    crate::history::list_records(&storage.pool(), page.unwrap_or(1), page_size.unwrap_or(10))
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn get_history_audio_waveform(
    history_id: String,
    samples: Option<u32>,
    storage: State<'_, SettingsDb>,
) -> Result<crate::history::AudioWaveform, String> {
    crate::history::waveform(&storage.pool(), &history_id, samples)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn delete_history_record(
    app: AppHandle,
    history_id: String,
    storage: State<'_, SettingsDb>,
) -> Result<crate::history::HistoryDeleteResult, String> {
    let result = crate::history::delete_record(&storage.pool(), &history_id)
        .await
        .map_err(command_error)?;
    emit_history_changed(&app);
    Ok(result)
}

#[tauri::command]
pub async fn save_history_record_export(
    history_id: String,
    destination_path: String,
    storage: State<'_, SettingsDb>,
) -> Result<HistoryExportSaveResult, String> {
    let record = crate::history::record(&storage.pool(), &history_id)
        .await
        .map_err(command_error)?
        .ok_or_else(|| "history record is not available".to_string())?;
    if !record
        .audio_file_path
        .as_deref()
        .is_some_and(|path| Path::new(path).is_file())
    {
        return Err("history record has no retained audio file".to_string());
    }

    let destination = zip_destination_path(destination_path);
    let write_path = destination.clone();
    tauri::async_runtime::spawn_blocking(move || {
        write_single_history_record_zip(&record, &write_path)
    })
    .await
    .map_err(|err| format!("history record export worker failed: {err}"))?
    .map_err(command_error)?;

    Ok(HistoryExportSaveResult {
        path: destination.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn start_history_export(
    app: AppHandle,
    storage: State<'_, SettingsDb>,
) -> Result<HistoryExportStart, String> {
    let export_id = Uuid::new_v4().to_string();
    let pool = storage.pool();
    let export_dir = storage.app_data_dir().join("exports");
    let temp_path = export_dir.join(format!("history-export-{export_id}.zip"));
    let cleanup_path = temp_path.clone();
    let task_app = app.clone();
    let task_export_id = export_id.clone();

    tauri::async_runtime::spawn(async move {
        if let Err(err) =
            run_history_export(task_app.clone(), task_export_id.clone(), pool, temp_path).await
        {
            log::warn!("history export failed export_id={task_export_id}: {err:#}");
            if let Err(remove_err) = fs::remove_file(&cleanup_path) {
                if cleanup_path.is_file() {
                    log::warn!(
                        "failed to remove incomplete history export {}: {remove_err}",
                        cleanup_path.display()
                    );
                }
            }
            emit_history_export_progress(
                &task_app,
                HistoryExportProgress {
                    export_id: task_export_id,
                    status: "error",
                    processed: 0,
                    total: 0,
                    message: Some(err.to_string()),
                },
            );
        }
    });

    Ok(HistoryExportStart { export_id })
}

#[tauri::command]
pub async fn save_history_export(
    export_id: String,
    destination_path: String,
) -> Result<HistoryExportSaveResult, String> {
    let temp_path = {
        let exports = HISTORY_EXPORTS
            .lock()
            .map_err(|_| "history export registry is unavailable".to_string())?;
        exports
            .get(&export_id)
            .cloned()
            .ok_or_else(|| "history export is not available".to_string())?
    };
    let destination = zip_destination_path(destination_path);

    fs::copy(&temp_path, &destination)
        .with_context(|| format!("copying export to {}", destination.display()))
        .map_err(command_error)?;
    if let Err(err) = fs::remove_file(&temp_path) {
        log::warn!("failed to remove temporary history export {}: {err}", temp_path.display());
    }
    if let Ok(mut exports) = HISTORY_EXPORTS.lock() {
        exports.remove(&export_id);
    }

    Ok(HistoryExportSaveResult {
        path: destination.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn delete_history_export(export_id: String) -> Result<(), String> {
    let temp_path = {
        let mut exports = HISTORY_EXPORTS
            .lock()
            .map_err(|_| "history export registry is unavailable".to_string())?;
        exports.remove(&export_id)
    };

    if let Some(path) = temp_path {
        if let Err(err) = fs::remove_file(&path) {
            if path.is_file() {
                log::warn!(
                    "failed to delete temporary history export {}: {err}",
                    path.display()
                );
            }
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn delete_all_history(
    app: AppHandle,
    storage: State<'_, SettingsDb>,
) -> Result<crate::history::HistoryDeleteResult, String> {
    let result = crate::history::delete_all(&storage.pool())
        .await
        .map_err(command_error)?;
    emit_history_changed(&app);
    Ok(result)
}

#[tauri::command]
pub async fn import_custom_sound(
    path: String,
    slot: String,
    storage: State<'_, SettingsDb>,
) -> Result<String, String> {
    let slot = match slot.as_str() {
        "start" | "stop" | "error" => slot,
        _ => return Err("invalid custom sound slot".to_string()),
    };
    let source = Path::new(&path);
    if !source.is_file() {
        return Err("selected sound file does not exist".to_string());
    }

    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case("wav") {
        return Err("selected sound file must be a WAV file".to_string());
    }
    hound::WavReader::open(source)
        .map_err(|err| format!("selected sound file is not a valid WAV file: {err}"))?;

    let custom_dir = storage.app_data_dir().join("sounds").join("custom").join(&slot);
    if custom_dir.exists() {
        std::fs::remove_dir_all(&custom_dir).map_err(|err| err.to_string())?;
    }
    std::fs::create_dir_all(&custom_dir).map_err(|err| err.to_string())?;

    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .map(sanitize_filename)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "custom.wav".to_string());
    let target = custom_dir.join(&file_name);
    std::fs::copy(source, &target).map_err(|err| err.to_string())?;

    Ok(format!("sounds/custom/{slot}/{file_name}"))
}

#[tauri::command]
pub async fn preview_sound(
    path: Option<String>,
    storage: State<'_, SettingsDb>,
) -> Result<(), String> {
    let path = path.ok_or_else(|| "no sound selected".to_string())?;
    let raw_path = std::path::PathBuf::from(&path);
    let resolved = if raw_path.is_absolute() {
        raw_path
    } else {
        storage.app_data_dir().join(raw_path)
    };
    let file = File::open(&resolved)
        .map_err(|err| format!("failed to open sound preview {}: {err}", resolved.display()))?;
    let source = rodio::Decoder::new(BufReader::new(file))
        .map_err(|err| format!("failed to decode sound preview {}: {err}", resolved.display()))?;
    let (_stream, handle) = rodio::OutputStream::try_default()
        .map_err(|err| format!("failed to initialize sound preview output: {err}"))?;
    let sink = rodio::Sink::try_new(&handle)
        .map_err(|err| format!("failed to create sound preview sink: {err}"))?;

    sink.append(source.convert_samples::<f32>());
    sink.sleep_until_end();
    Ok(())
}

#[tauri::command]
pub async fn list_model_settings(
    role: String,
    storage: State<'_, SettingsDb>,
) -> Result<RoleModelSettings, String> {
    let factory = crate::providers::ProviderFactory::new(storage.pool());
    factory.model_settings(&role).await.map_err(command_error)
}

#[tauri::command]
pub async fn list_model_health(
    role: String,
    health_cache: State<'_, ModelHealthCache>,
) -> Result<Vec<ModelHealthView>, String> {
    Ok(health_cache.role_snapshot(&role))
}

#[tauri::command]
pub async fn refresh_provider_models(
    role: String,
    provider_id: String,
    provider_config_override: Value,
    health_cache: State<'_, ModelHealthCache>,
    storage: State<'_, SettingsDb>,
) -> Result<Vec<ProviderModelOption>, String> {
    let factory = crate::providers::ProviderFactory::new(storage.pool());
    let options = factory
        .refresh_provider_models(&role, &provider_id, provider_config_override)
        .await
        .map_err(command_error)?;
    health_cache
        .sync_metadata(&storage.pool())
        .await
        .map_err(command_error)?;
    Ok(options)
}

#[tauri::command]
pub async fn save_model_item(
    user_model_id: Option<String>,
    role: String,
    provider_id: String,
    model_id: String,
    provider_config_override: Value,
    model_config_override: Value,
    app: AppHandle,
    health_cache: State<'_, ModelHealthCache>,
    storage: State<'_, SettingsDb>,
) -> Result<String, String> {
    let factory = crate::providers::ProviderFactory::new(storage.pool());
    let model_id = factory
        .save_model_item(
            user_model_id,
            &role,
            &provider_id,
            &model_id,
            provider_config_override,
            model_config_override,
        )
        .await
        .map_err(command_error)?;
    if let Err(err) = health_cache.refresh_model(&storage.pool(), &role, &model_id).await {
        log::warn!("failed to refresh model health after save model_id={model_id}: {err}");
    }
    refresh_tray_and_emit_model_health(&app);
    restart_runtime_after_provider_change(&app)?;
    Ok(model_id)
}

#[tauri::command]
pub async fn delete_model_item(
    role: String,
    model_id: String,
    app: AppHandle,
    health_cache: State<'_, ModelHealthCache>,
    storage: State<'_, SettingsDb>,
) -> Result<(), String> {
    let factory = crate::providers::ProviderFactory::new(storage.pool());
    factory
        .delete_model_item(&role, &model_id)
        .await
        .map_err(command_error)?;
    health_cache
        .sync_metadata(&storage.pool())
        .await
        .map_err(command_error)?;
    refresh_tray_and_emit_model_health(&app);
    restart_runtime_after_provider_change(&app)
}

#[tauri::command]
pub async fn select_model(
    role: String,
    model_id: String,
    app: AppHandle,
    health_cache: State<'_, ModelHealthCache>,
    storage: State<'_, SettingsDb>,
) -> Result<(), String> {
    let factory = crate::providers::ProviderFactory::new(storage.pool());
    factory
        .set_active_model(&role, &model_id)
        .await
        .map_err(command_error)?;
    health_cache
        .sync_metadata(&storage.pool())
        .await
        .map_err(command_error)?;
    refresh_tray_and_emit_model_health(&app);
    restart_runtime_after_provider_change(&app)
}

#[tauri::command]
pub async fn list_prompts(storage: State<'_, SettingsDb>) -> Result<Vec<PromptView>, String> {
    let pool = storage.pool();
    crate::prompts::list_prompts(&pool).await.map_err(command_error)
}

#[tauri::command]
pub async fn save_prompt(
    prompt_id: Option<String>,
    name: String,
    description: String,
    template: String,
    app: AppHandle,
    prompt_cache: State<'_, PromptCache>,
    storage: State<'_, SettingsDb>,
) -> Result<String, String> {
    let pool = storage.pool();
    let prompt_id = crate::prompts::save_prompt(&pool, prompt_id, name, description, template)
        .await
        .map_err(command_error)?;
    let is_active = crate::prompts::prompt_is_active(&pool, &prompt_id)
        .await
        .map_err(command_error)?;
    prompt_cache.sync(&pool).await.map_err(command_error)?;
    emit_prompts_changed(&app);
    if is_active {
        log::info!("active prompt saved; restarting runtime prompt_id={prompt_id}");
        restart_runtime_after_provider_change(&app)?;
    }
    Ok(prompt_id)
}

#[tauri::command]
pub async fn delete_prompt(
    prompt_id: String,
    app: AppHandle,
    prompt_cache: State<'_, PromptCache>,
    storage: State<'_, SettingsDb>,
) -> Result<(), String> {
    let pool = storage.pool();
    let was_active = crate::prompts::delete_prompt(&pool, &prompt_id)
        .await
        .map_err(command_error)?;
    prompt_cache.sync(&pool).await.map_err(command_error)?;
    emit_prompts_changed(&app);
    if was_active {
        log::info!("active prompt deleted; restarting runtime prompt_id={prompt_id}");
        restart_runtime_after_provider_change(&app)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn select_prompt(
    prompt_id: String,
    app: AppHandle,
    prompt_cache: State<'_, PromptCache>,
    storage: State<'_, SettingsDb>,
) -> Result<(), String> {
    let pool = storage.pool();
    crate::prompts::select_prompt(&pool, &prompt_id)
        .await
        .map_err(command_error)?;
    prompt_cache.sync(&pool).await.map_err(command_error)?;
    emit_prompts_changed(&app);
    log::info!("active prompt selected; restarting runtime prompt_id={prompt_id}");
    restart_runtime_after_provider_change(&app)
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

async fn run_history_export(
    app: AppHandle,
    export_id: String,
    pool: SqlitePool,
    temp_path: PathBuf,
) -> anyhow::Result<()> {
    let records = crate::history::all_records(&pool).await?;
    let total = records.len();
    emit_history_export_progress(
        &app,
        HistoryExportProgress {
            export_id: export_id.clone(),
            status: "packing",
            processed: 0,
            total,
            message: None,
        },
    );

    let zip_path = temp_path.clone();
    let progress_app = app.clone();
    let progress_export_id = export_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        write_history_export_zip(&records, &zip_path, |processed| {
            emit_history_export_progress(
                &progress_app,
                HistoryExportProgress {
                    export_id: progress_export_id.clone(),
                    status: "packing",
                    processed,
                    total,
                    message: None,
                },
            );
        })
    })
    .await
    .map_err(|err| anyhow!("history export worker failed: {err}"))??;

    {
        let mut exports = HISTORY_EXPORTS
            .lock()
            .map_err(|_| anyhow!("history export registry is unavailable"))?;
        exports.insert(export_id.clone(), temp_path);
    }

    emit_history_export_progress(
        &app,
        HistoryExportProgress {
            export_id,
            status: "complete",
            processed: total,
            total,
            message: None,
        },
    );

    Ok(())
}

fn write_history_export_zip<F>(
    records: &[crate::history::HistoryRecord],
    temp_path: &Path,
    mut on_progress: F,
) -> anyhow::Result<()>
where
    F: FnMut(usize),
{
    if let Some(parent) = temp_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating history export directory {}", parent.display()))?;
    }

    let file = File::create(temp_path)
        .with_context(|| format!("creating temporary history export {}", temp_path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for (index, record) in records.iter().enumerate() {
        let number = format!("{index:03}");
        let audio_file_name = export_audio_file_name(record, &number);
        if let (Some(source), Some(file_name)) = (
            record.audio_file_path.as_deref().filter(|path| Path::new(path).is_file()),
            audio_file_name.as_deref(),
        ) {
            zip.start_file(file_name, options)?;
            let mut audio = File::open(source)
                .with_context(|| format!("opening retained audio file {source}"))?;
            io::copy(&mut audio, &mut zip)?;
        }

        zip.start_file(format!("transcript_{number}.md"), options)?;
        zip.write_all(
            history_record_markdown(record, index, audio_file_name.as_deref()).as_bytes(),
        )?;
        on_progress(index + 1);
    }

    zip.finish()?;
    Ok(())
}

fn write_single_history_record_zip(
    record: &crate::history::HistoryRecord,
    destination: &Path,
) -> anyhow::Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating history export directory {}", parent.display()))?;
    }

    let source = record
        .audio_file_path
        .as_deref()
        .filter(|path| Path::new(path).is_file())
        .ok_or_else(|| anyhow!("history record has no retained audio file"))?;
    let file = File::create(destination)
        .with_context(|| format!("creating history record export {}", destination.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let audio_file_name = "transcript.wav";

    zip.start_file(audio_file_name, options)?;
    let mut audio =
        File::open(source).with_context(|| format!("opening retained audio file {source}"))?;
    io::copy(&mut audio, &mut zip)?;

    zip.start_file("transcript.md", options)?;
    zip.write_all(history_record_markdown(record, 0, Some(audio_file_name)).as_bytes())?;
    zip.finish()?;
    Ok(())
}

fn export_audio_file_name(record: &crate::history::HistoryRecord, number: &str) -> Option<String> {
    record
        .audio_file_path
        .as_deref()
        .filter(|path| Path::new(path).is_file())
        .map(|_| format!("transcript_{number}.wav"))
}

fn zip_destination_path(destination_path: String) -> PathBuf {
    let mut destination = PathBuf::from(destination_path);
    if destination.extension().and_then(|value| value.to_str()).is_none() {
        destination.set_extension("zip");
    }
    destination
}

fn history_record_markdown(
    record: &crate::history::HistoryRecord,
    index: usize,
    audio_file_name: Option<&str>,
) -> String {
    let transcript = record.transcript_text.as_deref().unwrap_or("_Unavailable_");
    let transform = record.transform_text.as_deref().unwrap_or("_Unavailable_");
    let error = record.error_message.as_deref().unwrap_or("_None_");
    let original_audio = record.audio_file_path.as_deref().unwrap_or("_Unavailable_");
    let exported_audio = audio_file_name.unwrap_or("_Unavailable_");

    format!(
        "# Transcript {index:03}\n\n\
         - ID: {id}\n\
         - Created at: {created_at}\n\
         - Duration: {duration}\n\
         - Audio duration ms: {audio_duration_ms}\n\
         - Original audio path: {original_audio}\n\
         - Exported audio file: {exported_audio}\n\
         - Error: {error}\n\n\
         ## Transcript\n\n\
         {transcript}\n\n\
         ## Transform\n\n\
         {transform}\n",
        id = record.id,
        created_at = record.created_at,
        duration = format_duration_ms(record.audio_duration_ms),
        audio_duration_ms = record.audio_duration_ms,
    )
}

fn format_duration_ms(value: u64) -> String {
    let total_seconds = (value + 500) / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes}:{seconds:02}")
}

fn emit_history_export_progress(app: &AppHandle, payload: HistoryExportProgress) {
    if let Err(err) = app.emit("history:export-progress", payload) {
        log::warn!("failed to emit history export progress: {err}");
    }
}

fn emit_history_changed(app: &AppHandle) {
    if let Err(err) = app.emit(
        "settings:changed",
        serde_json::json!({
            "source": "history",
            "keys": ["history.overview"],
        }),
    ) {
        log::warn!("failed to emit history change: {err}");
    }
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

fn restart_runtime_after_provider_change(app: &AppHandle) -> Result<(), String> {
    let Some(config) = crate::app_host::current_config(app) else {
        return Ok(());
    };
    crate::app_host::restart_runtime(app, config).map_err(|err| err.to_string())
}

fn refresh_tray_and_emit_model_health(app: &AppHandle) {
    if let Some(config) = crate::app_host::current_config(app) {
        crate::app_host::refresh_tray_menu(app, &config);
    }
    if let Err(err) = app.emit(
        "settings:changed",
        serde_json::json!({
            "source": "model_health",
            "keys": ["models.health", "models.active"],
        }),
    ) {
        log::warn!("failed to emit model health change: {err}");
    }
}

fn emit_prompts_changed(app: &AppHandle) {
    if let Err(err) = app.emit(
        "settings:changed",
        serde_json::json!({
            "source": "prompts",
            "keys": ["prompts.list", "prompts.active"],
        }),
    ) {
        log::warn!("failed to emit prompts change: {err}");
    }
}

fn sanitize_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn requires_runtime_restart(previous: Option<&AppConfig>, next: &AppConfig) -> bool {
    let Some(previous) = previous else {
        return true;
    };

    runtime_config_value(previous) != runtime_config_value(next)
}

fn runtime_config_value(config: &AppConfig) -> Value {
    serde_json::json!({
        "history_enabled": config.history_enabled,
        "provider": &config.provider,
        "interaction": &config.interaction,
        "pipeline": &config.pipeline,
        "recording": &config.recording,
        "models": &config.models,
        "audio": &config.audio,
        "audio_cues": &config.audio_cues,
        "output": &config.output,
    })
}

fn settings_fingerprint(settings: &Value) -> String {
    serde_json::json!({
        "system.history_enabled": settings.get("system.history_enabled"),
        "system.language": settings.get("system.language"),
        "recording.mode": settings.get("recording.mode"),
        "recording.hotkey": settings.get("recording.hotkey"),
        "recording.microphone.input_device": settings.get("recording.microphone.input_device"),
        "recording.pause_media": settings.get("recording.pause_media"),
        "models.formatting.enabled": settings.get("models.formatting.enabled"),
        "recording.sounds.start": settings.get("recording.sounds.start"),
        "recording.sounds.stop": settings.get("recording.sounds.stop"),
        "recording.sounds.error": settings.get("recording.sounds.error"),
    })
    .to_string()
}

fn config_fingerprint(config: &AppConfig) -> String {
    serde_json::json!({
        "history_enabled": config.history_enabled,
        "system.language": config.ui.language,
        "recording.mode": config.interaction.mode,
        "recording.hotkey": config.interaction.shortcut,
        "recording.microphone.input_device": config.audio.input_device,
        "recording.pause_media": config.recording.pause_media,
        "models.formatting.enabled": config.models.formatting_enabled,
        "recording.sounds.start": config.audio_cues.start_sound,
        "recording.sounds.stop": config.audio_cues.stop_sound,
        "recording.sounds.error": config.audio_cues.error_sound,
    })
    .to_string()
}
