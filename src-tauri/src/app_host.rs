use crate::audio_cues;
use crate::commands::settings;
use crate::config::AppConfig;
use crate::contracts::commands::RecordingCommand;
use crate::contracts::events::HotkeyEvent;
use crate::contracts::status::SessionStatusReceiver;
use crate::model_health::ModelHealthCache;
use crate::prompts;
use crate::providers::ProviderFactory;
use crate::recording;
use crate::recording::command_bus::CommandBusTx;
use crate::recording::workers::processor_worker::TransformRuntime;
use crate::settings_db::{SettingsDb, SETTINGS_DB_URL};
use crate::tray::{self, AppTray};
use anyhow::{anyhow, Result};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_log::{Target, TargetKind};
use tauri_plugin_sql::{Migration, MigrationKind};

const SETTINGS_WINDOW_LABEL: &str = "settings";

#[cfg(target_os = "macos")]
pub fn show_dock_icon(app: &AppHandle) {
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    log::debug!("dock icon shown with ActivationPolicy::Regular");
}

#[cfg(target_os = "macos")]
pub fn hide_dock_icon(app: &AppHandle) {
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    log::debug!("dock icon hidden with ActivationPolicy::Accessory");
}

#[derive(Clone)]
pub struct AppRuntimeState {
    inner: Arc<Mutex<Option<AppRuntime>>>,
}

struct AppRuntime {
    cfg: AppConfig,
    bus_tx: CommandBusTx,
    status_task: tauri::async_runtime::JoinHandle<()>,
    orchestrator: tokio::task::JoinHandle<Result<()>>,
    shortcut: Option<Shortcut>,
}

impl AppRuntimeState {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    fn runtime_slot(&self) -> Arc<Mutex<Option<AppRuntime>>> {
        self.inner.clone()
    }
}

fn settings_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            description: "create_current_settings_schema",
            sql: "CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS providers (
                id TEXT PRIMARY KEY,
                key TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                config_json TEXT NOT NULL,
                config_schema_json TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                is_preset INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS models (
                id TEXT PRIMARY KEY,
                provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
                role TEXT NOT NULL CHECK(role IN ('stt', 'formatting')),
                external_model_id TEXT NOT NULL,
                display_name TEXT NOT NULL,
                config_json TEXT NOT NULL,
                config_schema_json TEXT NOT NULL,
                adapter_config_json TEXT NOT NULL,
                adapter_config_schema_json TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                is_preset INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(provider_id, external_model_id, role)
            );
            CREATE INDEX IF NOT EXISTS models_provider_role_idx ON models(provider_id, role);
            CREATE TABLE IF NOT EXISTS user_models (
                id TEXT PRIMARY KEY,
                role TEXT NOT NULL CHECK(role IN ('stt', 'formatting')),
                provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE RESTRICT,
                model_id TEXT NOT NULL REFERENCES models(id) ON DELETE RESTRICT,
                provider_config_override_json TEXT NOT NULL,
                model_config_override_json TEXT NOT NULL,
                is_active INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS user_models_role_idx ON user_models(role);
            CREATE INDEX IF NOT EXISTS user_models_provider_model_idx ON user_models(provider_id, model_id);
            CREATE UNIQUE INDEX IF NOT EXISTS user_models_active_role_idx
                ON user_models(role)
                WHERE is_active = 1;
            CREATE TABLE IF NOT EXISTS prompts (
                id TEXT PRIMARY KEY,
                key TEXT UNIQUE,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                template TEXT NOT NULL,
                is_active INTEGER NOT NULL DEFAULT 0,
                is_preset INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS prompts_active_idx
                ON prompts(is_active)
                WHERE is_active = 1;",
            kind: MigrationKind::Up,
        },
    ]
}

pub fn run() -> Result<()> {
    let runtime_state = AppRuntimeState::new();
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_app, argv, cwd| {
            log::info!(
                "second app launch ignored argv={} cwd={}",
                argv.join(" "),
                cwd
            );
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Debug)
                .filter(|metadata| {
                    if metadata.target().starts_with("sqlx")
                        && matches!(
                            metadata.level(),
                            log::Level::Trace | log::Level::Debug | log::Level::Info
                        )
                    {
                        return false;
                    }
                    true
                })
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir {
                        file_name: Some("app".into()),
                    }),
                    Target::new(TargetKind::Webview),
                ])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations(SETTINGS_DB_URL, settings_migrations())
                .build(),
        )
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(handle_global_shortcut)
                .build(),
        );

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_plugin_macos_permissions::init());
    }

    builder
        .manage(runtime_state.clone())
        .invoke_handler(tauri::generate_handler![
            settings::apply_settings,
            settings::get_config_path,
            settings::open_config_file,
            settings::list_input_devices,
            settings::list_sound_options,
            settings::import_custom_sound,
            settings::preview_sound,
            settings::list_model_settings,
            settings::list_model_health,
            settings::refresh_provider_models,
            settings::save_model_item,
            settings::delete_model_item,
            settings::select_model,
            settings::list_prompts,
            settings::save_prompt,
            settings::delete_prompt,
            settings::select_prompt,
            settings::check_permissions,
            settings::request_microphone_permission,
            settings::request_accessibility_permission,
        ])
        .setup(|app| {
            let app_handle = app.handle().clone();
            #[cfg(target_os = "macos")]
            {
                hide_dock_icon(&app_handle);
                log::info!("set macOS activation policy to Accessory");
            }
            let storage = tauri::async_runtime::block_on(SettingsDb::connect(&app_handle))
                .map_err(|err| anyhow!(err.to_string()))?;
            let cfg = tauri::async_runtime::block_on(storage.load_config())
                .map_err(|err| anyhow!(err.to_string()))?;
            log::info!(
                "starting app v{} with interaction {:?} and shortcut {}",
                env!("CARGO_PKG_VERSION"),
                cfg.interaction.mode,
                cfg.interaction.shortcut
            );
            log::info!("settings database path: {}", storage.db_path().display());
            let health_cache = ModelHealthCache::new();
            tauri::async_runtime::block_on(health_cache.sync_metadata(&storage.pool()))
                .map_err(|err| anyhow!(err.to_string()))?;
            let health_pool = storage.pool();
            let health_cache_task = health_cache.clone();
            let health_app = app_handle.clone();
            app.manage(storage);
            app.manage(health_cache);
            let tray = tray::create_tray(&app_handle, &cfg)
                .map_err(|err| anyhow!(err.to_string()))?;
            app.manage(tray);
            tauri::async_runtime::spawn(async move {
                log::info!("model health startup refresh started");
                if let Err(err) = health_cache_task.refresh_all(&health_pool).await {
                    log::warn!("model health startup refresh failed: {err}");
                    return;
                }
                if let Err(err) = health_app.emit(
                    "settings:changed",
                    serde_json::json!({
                        "source": "model_health",
                        "keys": ["models.health"],
                    }),
                ) {
                    log::warn!("failed to emit startup model health change: {err}");
                }
                log::info!("model health startup refresh finished");
            });
            let runtime_cfg = cfg.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = start_runtime(&app_handle, runtime_cfg).await {
                    log::error!("failed to start runtime: {err}");
                }
            });
            Ok(())
        })
        .on_menu_event(|app, event| {
            handle_menu_event(app, event.id().as_ref());
        })
        .on_window_event(|window, event| {
            if window.label() != SETTINGS_WINDOW_LABEL {
                return;
            }

            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(err) = window.hide() {
                    log::warn!("failed to hide settings window on close: {err}");
                } else {
                    log::info!("settings window hidden instead of closed");
                    #[cfg(target_os = "macos")]
                    hide_dock_icon(window.app_handle());
                }
            }
        })
        .build(tauri::generate_context!())
        .map_err(|err| anyhow!(err.to_string()))?
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { has_visible_windows, .. } = event {
                if !has_visible_windows {
                    if let Err(err) = open_settings_window(app) {
                        log::warn!("failed to reopen settings window: {err}");
                    }
                }
            }
        });

    Ok(())
}

pub fn open_settings_window(app: &AppHandle) -> Result<()> {
    let window = app
        .get_webview_window(SETTINGS_WINDOW_LABEL)
        .ok_or_else(|| anyhow!("settings window not found"))?;

    #[cfg(target_os = "macos")]
    show_dock_icon(app);

    window.show().map_err(|err| anyhow!(err.to_string()))?;
    window.unminimize().map_err(|err| anyhow!(err.to_string()))?;
    window.set_focus().map_err(|err| anyhow!(err.to_string()))?;
    Ok(())
}

pub fn restart_runtime(app: &AppHandle, cfg: AppConfig) -> Result<()> {
    log::info!(
        "runtime restart requested fingerprint={}",
        runtime_fingerprint(&cfg)
    );
    stop_runtime(app);
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(err) = start_runtime(&app_handle, cfg).await {
            log::error!("failed to restart runtime: {err}");
        }
    });
    Ok(())
}

pub fn refresh_tray_menu(app: &AppHandle, config: &AppConfig) {
    if let Some(tray) = app.try_state::<AppTray>() {
        tray.refresh_menu(app, config);
    } else {
        log::warn!("tray refresh requested before tray state was available");
    }
}

async fn start_runtime(app: &AppHandle, cfg: AppConfig) -> Result<()> {
    cfg.validate()?;
    log::info!("runtime start applying fingerprint={}", runtime_fingerprint(&cfg));
    let runtime_state = app.state::<AppRuntimeState>().runtime_slot();
    let tray = app.state::<AppTray>().inner().clone();
    let cue = audio_cues::CuePlayer::new(&cfg.audio_cues, &cfg);
    cue.run_self_test_if_requested();
    let (stt_provider, transform) = match app.try_state::<SettingsDb>() {
        Some(storage) => {
            let pool = storage.pool();
            let factory = match app.try_state::<ModelHealthCache>() {
                Some(health_cache) => ProviderFactory::with_health(
                    pool.clone(),
                    health_cache.clone_cache(),
                    app.clone(),
                ),
                None => ProviderFactory::new(pool.clone()),
            };
            let stt_provider = match factory.speech_to_text().await {
                Ok(provider) => provider,
                Err(err) => {
                    log::warn!("provider factory error: {err}");
                    None
                }
            };

            let transform = if cfg.models.formatting_enabled {
                let formatter = match factory.formatting().await {
                    Ok(provider) => provider,
                    Err(err) => {
                        log::warn!("formatting provider factory error: {err}");
                        None
                    }
                };
                let prompt = match prompts::active_prompt(&pool).await {
                    Ok(prompt) => prompt,
                    Err(err) => {
                        log::warn!("active transform prompt load failed: {err}");
                        None
                    }
                };
                let health = app
                    .try_state::<ModelHealthCache>()
                    .and_then(|health_cache| {
                        health_cache
                            .role_snapshot("formatting")
                            .into_iter()
                            .find(|model| model.is_active)
                            .map(|model| model.health)
                    });
                if formatter.is_none() {
                    log::warn!("transform enabled but no formatting model is configured");
                }
                if prompt.is_none() {
                    log::warn!("transform enabled but no active prompt is configured");
                }
                TransformRuntime::new(true, formatter, prompt, health)
            } else {
                TransformRuntime::disabled()
            };

            (stt_provider, transform)
        }
        None => {
            log::warn!("provider factory unavailable because settings db state is missing");
            (None, TransformRuntime::disabled())
        }
    };

    if stt_provider.is_none() {
        log::warn!("speech-to-text provider not configured");
    }

    let (bus_tx, status_rx, orchestrator) =
        recording::orchestrator::start_with_transform(cfg.clone(), cue.clone(), stt_provider, transform);
    let status_task = spawn_status_task(status_rx, tray, cue, cfg.clone());
    let shortcut = register_shortcut(app, &cfg)?;

    let runtime = AppRuntime {
        cfg,
        bus_tx,
        status_task,
        orchestrator,
        shortcut,
    };

    if let Ok(mut guard) = runtime_state.lock() {
        *guard = Some(runtime);
    }

    Ok(())
}

fn stop_runtime(app: &AppHandle) {
    let state = app.state::<AppRuntimeState>();
    let runtime = state
        .inner
        .lock()
        .ok()
        .and_then(|mut guard| guard.take());

    if let Some(runtime) = runtime {
        if let Some(shortcut) = runtime.shortcut {
            if let Err(err) = app.global_shortcut().unregister(shortcut) {
                log::warn!("failed to unregister shortcut: {err}");
            }
        }
        let _ = runtime.bus_tx.send_command(RecordingCommand::Shutdown);
        runtime.status_task.abort();
        runtime.orchestrator.abort();
    }
}

pub fn current_config(app: &AppHandle) -> Option<AppConfig> {
    let state = app.try_state::<AppRuntimeState>()?;
    state
        .inner
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|runtime| runtime.cfg.clone()))
}

fn register_shortcut(app: &AppHandle, cfg: &AppConfig) -> Result<Option<Shortcut>> {
    let shortcut: Shortcut = cfg
        .interaction
        .shortcut
        .parse()
        .map_err(|err| anyhow!("invalid shortcut '{}': {err}", cfg.interaction.shortcut))?;

    app.global_shortcut()
        .register(shortcut)
        .map_err(|err| anyhow!("failed to register shortcut '{}': {err}", cfg.interaction.shortcut))?;

    log::info!("registered global shortcut: {}", cfg.interaction.shortcut);
    Ok(Some(shortcut))
}

fn runtime_fingerprint(cfg: &AppConfig) -> String {
    serde_json::json!({
        "language": cfg.ui.language,
        "mode": cfg.interaction.mode,
        "shortcut": cfg.interaction.shortcut,
        "input_device": cfg.audio.input_device,
        "pause_media": cfg.recording.pause_media,
        "formatting_enabled": cfg.models.formatting_enabled,
        "start_sound": cfg.audio_cues.start_sound,
        "stop_sound": cfg.audio_cues.stop_sound,
        "error_sound": cfg.audio_cues.error_sound,
    })
    .to_string()
}

fn handle_global_shortcut(
    app: &AppHandle,
    shortcut: &Shortcut,
    event: tauri_plugin_global_shortcut::ShortcutEvent,
) {
    let Some(runtime_state) = app.try_state::<AppRuntimeState>() else {
        return;
    };

    let Some((cfg, bus_tx, registered)) = runtime_state.inner.lock().ok().and_then(|guard| {
        guard.as_ref().map(|runtime| {
            (
                runtime.cfg.clone(),
                runtime.bus_tx.clone(),
                runtime.shortcut,
            )
        })
    }) else {
        return;
    };

    if registered.as_ref() != Some(shortcut) {
        return;
    }

    let hotkey = match event.state() {
        ShortcutState::Pressed => HotkeyEvent::Pressed,
        ShortcutState::Released => HotkeyEvent::Released,
    };

    log::debug!(
        "global shortcut event mode={:?} shortcut={:?} state={:?} hotkey={}",
        cfg.interaction.mode,
        shortcut,
        event.state(),
        hotkey
    );
    if let Some(worker_event) = bus_tx.send_hotkey(hotkey) {
        let _ = bus_tx.send_worker(worker_event);
    }
}

fn spawn_status_task(
    mut status_rx: SessionStatusReceiver,
    tray: AppTray,
    cue: audio_cues::CuePlayer,
    cfg: AppConfig,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let mut pulse_timer = tokio::time::interval(tokio::time::Duration::from_millis(
            cfg.tray.refresh_ms.max(1),
        ));
        pulse_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let _ = pulse_timer.tick().await;
        let mut is_recording = false;
        let mut last_cued_event: Option<String> = None;

        loop {
            tokio::select! {
                status = status_rx.recv() => {
                    let Some(status) = status else {
                        break;
                    };
                    let render = crate::runtime::render_status_for_host(&cfg.tray, &status);
                    is_recording = render.should_pulse;
                    tray.set_status(render.tooltip, render.icon_state);
                    if let Some(cue_kind) = render.cue {
                        let cue_key = format!("{}:{}", status.session_id, status.source);
                        if Some(cue_key.as_str()) != last_cued_event.as_deref() {
                            match cue_kind {
                                audio_cues::CueKind::Start if cfg.recording.pause_media => {
                                    log::debug!("cue skipped in status task because recording pause_media owns start cue");
                                }
                                audio_cues::CueKind::Start => cue.play_start(),
                                audio_cues::CueKind::Stop => cue.play_stop(),
                                audio_cues::CueKind::Error => cue.play_error(),
                            }
                            last_cued_event = Some(cue_key);
                        }
                    }
                }
                _ = pulse_timer.tick(), if is_recording => {
                    tray.pulse_recording();
                }
            }
        }
    })
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    if id == tray::MENU_SETTINGS {
        if let Err(err) = open_settings_window(app) {
            log::warn!("failed to open settings: {err}");
        }
    } else if id == tray::MENU_QUIT {
        stop_runtime(app);
        app.exit(0);
    } else if id == tray::MENU_DEVICE_DEFAULT {
        save_input_device_from_tray(app, None);
    } else if let Some(device) = id.strip_prefix(tray::MENU_DEVICE_PREFIX) {
        save_input_device_from_tray(app, Some(device.to_string()));
    } else if id == tray::MENU_MODEL_FORMATTING_DISABLE {
        save_formatting_enabled_from_tray(app, false);
    } else if let Some(selection) = id.strip_prefix(tray::MENU_MODEL_PREFIX) {
        if !selection.starts_with("stt:") && !selection.starts_with("formatting:") {
            return;
        }
        save_model_selection_from_tray(app, selection);
    }
}

fn save_model_selection_from_tray(app: &AppHandle, selection: &str) {
    let Some((role, model_id)) = selection.split_once(':') else {
        return;
    };
    let app = app.clone();
    let role = role.to_string();
    let model_id = model_id.to_string();
    tauri::async_runtime::spawn(async move {
        let Some(storage) = app.try_state::<SettingsDb>() else {
            log::warn!("failed to save model selection from tray: settings database unavailable");
            return;
        };
        let factory = ProviderFactory::new(storage.pool());
        if let Err(err) = factory.set_active_model(&role, &model_id).await {
            log::warn!("failed to save model selection from tray: {err}");
            return;
        }
        if role == "formatting" {
            let mut settings = match storage.load_settings().await {
                Ok(settings) => settings,
                Err(err) => {
                    log::warn!("failed to load settings for tray transform model selection: {err}");
                    return;
                }
            };
            settings["models.formatting.enabled"] = serde_json::Value::Bool(true);
            if let Err(err) = storage.save_settings(&settings).await {
                log::warn!("failed to save tray transform enable change: {err}");
                return;
            }
        }
        if let Some(health_cache) = app.try_state::<ModelHealthCache>() {
            if let Err(err) = health_cache.sync_metadata(&storage.pool()).await {
                log::warn!("failed to sync model health cache after tray model selection: {err}");
            }
        }
        let config = match storage.load_config().await {
            Ok(config) => config,
            Err(err) => {
                log::warn!("failed to load runtime config after tray model selection: {err}");
                return;
            }
        };
        refresh_tray_menu(&app, &config);
        if let Err(err) = restart_runtime(&app, config) {
                log::error!("failed to apply tray model selection: {err}");
        }
        if let Err(err) = app.emit(
            "settings:changed",
            serde_json::json!({
                "source": "tray",
                "keys": ["models.active", "models.formatting.enabled"],
            }),
        ) {
            log::warn!("failed to emit settings change after tray model selection: {err}");
        }
    });
}

fn save_formatting_enabled_from_tray(app: &AppHandle, enabled: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(storage) = app.try_state::<SettingsDb>() else {
            log::warn!("failed to save transform setting from tray: settings database unavailable");
            return;
        };
        let previous = current_config(&app);
        let mut settings = match storage.load_settings().await {
            Ok(settings) => settings,
            Err(err) => {
                log::warn!("failed to load settings for tray transform setting change: {err}");
                return;
            }
        };
        settings["models.formatting.enabled"] = serde_json::Value::Bool(enabled);
        if let Err(err) = storage.save_settings(&settings).await {
            log::warn!("failed to save tray transform setting change: {err}");
            return;
        }
        let config = match crate::settings_schema::runtime_config_from_settings(&settings) {
            Ok(config) => config,
            Err(err) => {
                log::warn!("failed to load runtime config after tray transform setting change: {err}");
                return;
            }
        };
        if let Err(err) = crate::commands::settings::apply_saved_config(&app, previous.as_ref(), &config, None) {
            log::error!("failed to apply tray transform setting change: {err}");
            return;
        }
        if let Err(err) = app.emit(
            "settings:changed",
            serde_json::json!({
                "source": "tray",
                "keys": ["models.formatting.enabled"],
            }),
        ) {
            log::warn!("failed to emit settings change after tray transform setting change: {err}");
        }
    });
}

fn save_input_device_from_tray(app: &AppHandle, device: Option<String>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(storage) = app.try_state::<SettingsDb>() else {
            log::warn!("failed to save input device from tray: settings database unavailable");
            return;
        };
        let previous = current_config(&app);
        let mut settings = match storage.load_settings().await {
            Ok(settings) => settings,
            Err(err) => {
                log::warn!("failed to load settings for tray input device change: {err}");
                return;
            }
        };
        settings["recording.microphone.input_device"] = device
            .clone()
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null);
        if let Err(err) = storage.save_settings(&settings).await {
            log::warn!("failed to save tray input device change: {err}");
            return;
        }
        let config = match crate::settings_schema::runtime_config_from_settings(&settings) {
            Ok(config) => config,
            Err(err) => {
                log::warn!("failed to load runtime config after tray input device change: {err}");
                return;
            }
        };
        if let Err(err) = crate::commands::settings::apply_saved_config(&app, previous.as_ref(), &config, None) {
            log::error!("failed to apply tray input device change: {err}");
            return;
        }
        if let Err(err) = app.emit(
            "settings:changed",
            serde_json::json!({
                "source": "tray",
                "keys": ["recording.microphone.input_device"],
            }),
        ) {
            log::warn!("failed to emit settings change after tray input device change: {err}");
        }
    });
}
