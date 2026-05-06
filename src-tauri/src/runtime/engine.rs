use crate::config::{AppConfig, TrayConfig};
use crate::contracts::events::PipelinePhase;
use crate::contracts::status::SessionStatus;
use crate::openrouter::OpenRouterClient;
use crate::{audio_cues, hotkey, recording, runtime::{compat, control}, tray};
use anyhow::{anyhow, Result};
use tokio::sync::mpsc;
use tokio::time::{interval, Duration, MissedTickBehavior};

#[cfg(not(target_os = "macos"))]
pub fn run_non_macos(cfg: AppConfig) -> Result<()> {
    cfg.validate()?;
    let (event_tx, event_rx) = mpsc::unbounded_channel::<compat::RuntimeControlEvent>();
    let config_path = AppConfig::config_path().to_string_lossy().into_owned();
    let tray = tray::TrayController::new(
        cfg.tray.title.clone(),
        cfg.tray.tooltip.idle.clone(),
        event_tx.clone(),
        config_path,
    )?;
    let tray_handle = tray.handle();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let cue = audio_cues::CuePlayer::new(&cfg.audio_cues, &cfg);
    cue.run_self_test_if_requested();
    runtime.block_on(run_core(cfg, event_tx, event_rx, cue, Some(tray_handle)))
}

#[cfg(target_os = "macos")]
pub fn run_macos(cfg: AppConfig) -> Result<()> {
    cfg.validate()?;
    let (event_tx, event_rx) = mpsc::unbounded_channel::<compat::RuntimeControlEvent>();
    let background_cfg = cfg.clone();
    let background_tx = event_tx.clone();
    let config_path = AppConfig::config_path().to_string_lossy().into_owned();
    let cue = audio_cues::CuePlayer::new(&cfg.audio_cues, &cfg);
    cue.run_self_test_if_requested();

    let tray = tray::TrayController::new(
        cfg.tray.title.clone(),
        cfg.tray.tooltip.idle.clone(),
        event_tx,
        config_path,
    )?;
    let tray_handle = tray.handle();

    let app_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to start macOS background runtime");
        let _ = rt.block_on(run_core_macos(
            background_cfg,
            background_tx,
            event_rx,
            cue,
            Some(tray_handle),
        ));
    });

    tray.run();
    let _ = app_thread.join();
    Ok(())
}

#[cfg(target_os = "macos")]
async fn run_core_macos(
    cfg: AppConfig,
    event_tx: mpsc::UnboundedSender<compat::RuntimeControlEvent>,
    event_rx: mpsc::UnboundedReceiver<compat::RuntimeControlEvent>,
    cue: audio_cues::CuePlayer,
    tray: Option<tray::TrayHandle>,
) -> Result<()> {
    run_core(cfg, event_tx, event_rx, cue, tray).await
}

async fn run_core(
    cfg: AppConfig,
    event_tx: mpsc::UnboundedSender<compat::RuntimeControlEvent>,
    event_rx: mpsc::UnboundedReceiver<compat::RuntimeControlEvent>,
    cue: audio_cues::CuePlayer,
    tray: Option<tray::TrayHandle>,
) -> Result<()> {
    let mut lifecycle = RuntimeLifecycle::start(cfg, event_tx, event_rx, cue)?;
    let mut pulse_timer =
        interval(Duration::from_millis(lifecycle.cfg.tray.refresh_ms.max(1)));
    pulse_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let _ = pulse_timer.tick().await;

    let mut is_recording = false;
    let mut latest_phase = PipelinePhase::Idle;
    let mut reload_pending = false;

    loop {
        tokio::select! {
            status = lifecycle.status_rx.recv() => {
                match status {
                    Some(status) => {
                        latest_phase = status.state;
                        let render = render_status(&lifecycle.cfg.tray, &status);
                        is_recording = render.should_pulse;
                        lifecycle.publish_status(status, render, tray.as_ref());
                        if reload_pending && latest_phase == PipelinePhase::Idle {
                            lifecycle.reload_runtime();
                        }
                    }
                    None => {
                        return lifecycle.wait_orchestrator().await;
                    }
                }
            }
            control = lifecycle.event_rx.recv() => {
                match control {
                    Some(compat::RuntimeControlEvent::Quit) => {
                        lifecycle.request_shutdown();
                    }
                    Some(compat::RuntimeControlEvent::ReloadRuntime) => {
                        if latest_phase == PipelinePhase::Idle {
                            lifecycle.reload_runtime();
                        } else {
                            reload_pending = true;
                            log::info!("runtime reload deferred until idle; current_phase={latest_phase:?}");
                        }
                    }
                    None => {}
                }
            }
            _ = pulse_timer.tick(), if is_recording && tray.is_some() => {
                if let Some(tray) = tray.as_ref() {
                    pulse_tray_recording(tray);
                }
            }
            result = &mut lifecycle.orchestrator => {
                return result?;
            }
        }
    }
}

struct RuntimeLifecycle {
    cfg: AppConfig,
    cue: audio_cues::CuePlayer,
    event_rx: mpsc::UnboundedReceiver<compat::RuntimeControlEvent>,
    bus_tx: crate::recording::command_bus::CommandBusTx,
    status_rx: crate::contracts::status::SessionStatusReceiver,
    orchestrator: tokio::task::JoinHandle<Result<()>>,
    last_cued_event: Option<String>,
    _runtime_control_server: std::thread::JoinHandle<()>,
    _hotkey_handle: std::thread::JoinHandle<()>,
    _signal_handle: tokio::task::JoinHandle<()>,
}

impl RuntimeLifecycle {
    fn start(
        cfg: AppConfig,
        event_tx: mpsc::UnboundedSender<compat::RuntimeControlEvent>,
        event_rx: mpsc::UnboundedReceiver<compat::RuntimeControlEvent>,
        cue: audio_cues::CuePlayer,
    ) -> Result<Self> {
    let openrouter = match OpenRouterClient::new(cfg.provider.openrouter.clone()) {
        Ok(client) => Some(client),
        Err(err) => {
            log::warn!("provider config error: {err}");
            None
        }
    };
        let (bus_tx, status_rx, orchestrator) =
        recording::orchestrator::start(cfg.clone(), cue.clone(), openrouter);
        let runtime_control_server = control::spawn_runtime_control_server(event_tx.clone())?;
        let mut interaction = cfg.interaction.clone();
        interaction.repeat_debounce_ms = cfg.effective_repeat_debounce_ms();
        let hotkey_handle = hotkey::spawn_listener(interaction, bus_tx.clone())?;

        let signal_handle = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = event_tx.send(compat::RuntimeControlEvent::Quit);
    });

        Ok(Self {
            cfg,
            cue,
            event_rx,
            bus_tx,
            status_rx,
            orchestrator,
            last_cued_event: None,
            _runtime_control_server: runtime_control_server,
            _hotkey_handle: hotkey_handle,
            _signal_handle: signal_handle,
        })
    }

    fn publish_status(
        &mut self,
        status: SessionStatus,
        render: StatusRender,
        tray: Option<&tray::TrayHandle>,
    ) {
        if let Some(tray) = tray {
            set_tray_status(tray, render.tooltip, render.icon_state);
        } else {
            log::info!("orchestrator runtime_status: {}", status);
        }

        if let Some(cue_kind) = render.cue {
            if Some(status.source.as_str()) != self.last_cued_event.as_deref() {
                match cue_kind {
                    audio_cues::CueKind::Start => self.cue.play_start(),
                    audio_cues::CueKind::Stop => self.cue.play_stop(),
                    audio_cues::CueKind::Error => self.cue.play_error(),
                }
                self.last_cued_event = Some(status.source.clone());
            }
        }
    }

    async fn wait_orchestrator(self) -> Result<()> {
        match self.orchestrator.await {
            Ok(result) => result,
            Err(err) => Err(anyhow!("orchestrator task stopped unexpectedly: {err}")),
        }
    }

    fn request_shutdown(&self) {
        let _ = self.bus_tx.send_command(crate::contracts::commands::RecordingCommand::Shutdown);
    }

    fn reload_runtime(&self) -> ! {
        log::info!("reloading runtime by spawning replacement process");
        match std::env::current_exe() {
            Ok(exe) => {
                if let Err(err) = std::process::Command::new(exe).spawn() {
                    log::error!("failed to spawn replacement runtime: {err}");
                    std::process::exit(1);
                }
            }
            Err(err) => {
                log::error!("failed to locate current executable for reload: {err}");
                std::process::exit(1);
            }
        }
        std::process::exit(0);
    }
}

fn set_tray_status(tray: &tray::TrayHandle, tooltip: String, icon_state: &'static str) {
    tray.set_status(tooltip, icon_state);
}

fn pulse_tray_recording(tray: &tray::TrayHandle) {
    tray.pulse_recording();
}

#[derive(Clone)]
struct StatusRender {
    tooltip: String,
    should_pulse: bool,
    icon_state: &'static str,
    cue: Option<audio_cues::CueKind>,
}

fn render_status(cfg: &TrayConfig, status: &SessionStatus) -> StatusRender {
    let icon_state = tray::status_to_icon(status);
    let should_pulse =
        matches!(status.state, PipelinePhase::Starting | PipelinePhase::Recording);
    let tooltip = match status.state {
        PipelinePhase::Idle => {
            if status.source == "processing_completed" {
                cfg.tooltip.success.clone()
            } else if !status.source.is_empty() && status.error_code.is_none() {
                status.source.clone()
            } else {
                cfg.tooltip.idle.clone()
            }
        }
        PipelinePhase::Starting | PipelinePhase::Recording => cfg.tooltip.recording.clone(),
        PipelinePhase::Stopping | PipelinePhase::Processing => cfg.tooltip.sending.clone(),
        PipelinePhase::Recovering => {
            if status.source.is_empty() {
                "recovering".to_string()
            } else {
                status.source.clone()
            }
        }
        PipelinePhase::Error => {
            if status.source.is_empty() {
                "recording error".to_string()
            } else {
                status.source.clone()
            }
        }
    };

    StatusRender {
        tooltip,
        should_pulse,
        icon_state,
        cue: audio_cues::CuePlayer::status_to_cue(status),
    }
}
