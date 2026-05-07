use crate::audio_cues;
use crate::commands::settings;
use crate::config::{AppConfig, InteractionMode};
use crate::contracts::commands::RecordingCommand;
use crate::contracts::events::HotkeyEvent;
use crate::contracts::status::SessionStatusReceiver;
use crate::openrouter::OpenRouterClient;
use crate::recording;
use crate::recording::command_bus::CommandBusTx;
use crate::tray::{self, AppTray};
use anyhow::{anyhow, Result};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const SETTINGS_WINDOW_LABEL: &str = "settings";
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

pub fn run(cfg: AppConfig) -> Result<()> {
    let runtime_state = AppRuntimeState::new();
    let settings_state = settings::build_settings_state().map_err(anyhow::Error::msg)?;
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
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
        .manage(settings_state)
        .invoke_handler(tauri::generate_handler![
            settings::load_config,
            settings::save_config,
            settings::get_config_path,
            settings::open_config_file,
            settings::get_settings,
            settings::update_settings,
            settings::check_permissions,
            settings::request_microphone_permission,
            settings::request_accessibility_permission,
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
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
                }
            }
        })
        .run(tauri::generate_context!())
        .map_err(|err| anyhow!(err.to_string()))?;

    Ok(())
}

pub fn open_settings_window(app: &AppHandle) -> Result<()> {
    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return Ok(());
    }

    WebviewWindowBuilder::new(app, SETTINGS_WINDOW_LABEL, settings_url())
        .title("Cretar IA Settings")
        .inner_size(880.0, 490.0)
        .resizable(true)
        .build()
        .map(|_| ())
        .map_err(|err| anyhow!(err.to_string()))
}

pub fn restart_runtime(app: &AppHandle, cfg: AppConfig) -> Result<()> {
    stop_runtime(app);
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(err) = start_runtime(&app_handle, cfg) {
            log::error!("failed to restart runtime: {err}");
        }
    });
    Ok(())
}

fn start_runtime(app: &AppHandle, cfg: AppConfig) -> Result<()> {
    cfg.validate()?;
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

    let hotkey = match cfg.interaction.mode {
        InteractionMode::Toggle if event.state() == ShortcutState::Pressed => {
            Some(HotkeyEvent::TogglePressed)
        }
        InteractionMode::PushToTalk if event.state() == ShortcutState::Pressed => {
            Some(HotkeyEvent::Pressed)
        }
        InteractionMode::PushToTalk if event.state() == ShortcutState::Released => {
            Some(HotkeyEvent::Released)
        }
        _ => None,
    };

    if let Some(hotkey) = hotkey {
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
    } else if let Some(device) = id.strip_prefix(tray::MENU_DEVICE_PREFIX) {
        if let Err(err) = tray::save_selected_input_device(device) {
            log::warn!("failed to save input device '{device}': {err}");
            return;
        }
        match AppConfig::load_or_create() {
            Ok(cfg) => {
                if let Some(tray) = app.try_state::<AppTray>() {
                    tray.refresh_menu(app);
                }
                if let Err(err) = restart_runtime(app, cfg) {
                    log::error!("failed to restart runtime after input device change: {err}");
                }
            }
            Err(err) => log::error!("failed to reload config after input device change: {err}"),
        }
    }
}

fn settings_url() -> WebviewUrl {
    #[cfg(debug_assertions)]
    {
        if let Ok(url) = "http://localhost:5173".parse() {
            return WebviewUrl::External(url);
        }
    }

    WebviewUrl::App("index.html".into())
}
