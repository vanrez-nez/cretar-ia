use crate::audio_cues;
use crate::commands::settings;
use crate::config::AppConfig;
use crate::contracts::commands::RecordingCommand;
use crate::contracts::events::HotkeyEvent;
use crate::contracts::status::SessionStatusReceiver;
use crate::openrouter::OpenRouterClient;
use crate::recording;
use crate::recording::command_bus::CommandBusTx;
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
    vec![Migration {
        version: 1,
        description: "create_settings_table",
        sql: "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
        kind: MigrationKind::Up,
    }]
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
            app.manage(storage);
            let tray = tray::create_tray(&app_handle, &cfg)
                .map_err(|err| anyhow!(err.to_string()))?;
            app.manage(tray);
            let runtime_cfg = cfg.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = start_runtime(&app_handle, runtime_cfg) {
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
        if let Err(err) = start_runtime(&app_handle, cfg) {
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

fn start_runtime(app: &AppHandle, cfg: AppConfig) -> Result<()> {
    cfg.validate()?;
    log::info!("runtime start applying fingerprint={}", runtime_fingerprint(&cfg));
    let runtime_state = app.state::<AppRuntimeState>().runtime_slot();
    let tray = app.state::<AppTray>().inner().clone();
    let cue = audio_cues::CuePlayer::new(&cfg.audio_cues, &cfg);
    cue.run_self_test_if_requested();
    let openrouter = match OpenRouterClient::new(cfg.provider.openrouter.clone()) {
        Ok(client) => Some(client),
        Err(err) => {
            log::warn!("provider config error: {err}");
            None
        }
    };

    let (bus_tx, status_rx, orchestrator) =
        recording::orchestrator::start(cfg.clone(), cue.clone(), openrouter);
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
        ShortcutState::Pressed => Some(HotkeyEvent::Pressed),
        ShortcutState::Released => Some(HotkeyEvent::Released),
        _ => None,
    };

    if let Some(hotkey) = hotkey {
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
                        if Some(status.source.as_str()) != last_cued_event.as_deref() {
                            match cue_kind {
                                audio_cues::CueKind::Start if cfg.recording.pause_media => {
                                    log::debug!("cue skipped in status task because recording pause_media owns start cue");
                                }
                                audio_cues::CueKind::Start => cue.play_start(),
                                audio_cues::CueKind::Stop => cue.play_stop(),
                                audio_cues::CueKind::Error => cue.play_error(),
                            }
                            last_cued_event = Some(status.source.clone());
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
    }
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
