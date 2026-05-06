use crate::config::{AppConfig, TrayConfig};
use crate::contracts::events::PipelinePhase;
use crate::contracts::status::SessionStatus;
use crate::openrouter::OpenRouterClient;
use crate::{audio_cues, hotkey, recording, runtime::compat, tray};
use anyhow::{anyhow, Result};
use tokio::sync::mpsc;
use tokio::time::{interval, Duration, MissedTickBehavior};

#[cfg(not(target_os = "macos"))]
pub fn run_non_macos(cfg: AppConfig) -> Result<()> {
    cfg.validate()?;
    let (event_tx, event_rx) = mpsc::unbounded_channel::<compat::RuntimeControlEvent>();
    let config_path = AppConfig::config_path().to_string_lossy().into_owned();
    let mut tray = tray::TrayController::new(
        cfg.tray.title.clone(),
        cfg.tray.tooltip.idle.clone(),
        event_tx.clone(),
        config_path,
    )?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let cue = audio_cues::CuePlayer::new(&cfg.audio_cues, &cfg);
    cue.run_self_test_if_requested();
    runtime.block_on(run_core(cfg, event_tx, event_rx, cue, Some(&mut tray)))
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

    let app_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to start macOS background runtime");
        let _ = rt.block_on(run_core_macos(background_cfg, background_tx, event_rx, cue));
    });

    let tray = tray::TrayController::new(
        cfg.tray.title.clone(),
        cfg.tray.tooltip.idle.clone(),
        event_tx,
        config_path,
    )?;

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
) -> Result<()> {
    run_core(cfg, event_tx, event_rx, cue, None).await
}

async fn run_core(
    cfg: AppConfig,
    event_tx: mpsc::UnboundedSender<compat::RuntimeControlEvent>,
    event_rx: mpsc::UnboundedReceiver<compat::RuntimeControlEvent>,
    cue: audio_cues::CuePlayer,
    mut tray: Option<&mut tray::TrayController>,
) -> Result<()> {
    let mut lifecycle = RuntimeLifecycle::start(cfg, event_tx, event_rx, cue)?;
    let mut pulse_timer =
        interval(Duration::from_millis(lifecycle.cfg.tray.refresh_ms.max(1)));
    pulse_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let _ = pulse_timer.tick().await;

    let mut is_recording = false;

    loop {
        tokio::select! {
            status = lifecycle.status_rx.recv() => {
                match status {
                    Some(status) => {
                        let render = render_status(&lifecycle.cfg.tray, &status);
                        is_recording = render.should_pulse;
                        lifecycle.publish_status(status, render, tray.as_deref_mut());
                    }
                    None => {
                        return lifecycle.wait_orchestrator().await;
                    }
                }
            }
            _ = pulse_timer.tick(), if is_recording && tray.is_some() => {
                if let Some(tray) = tray.as_deref_mut() {
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
    status_rx: crate::contracts::status::SessionStatusReceiver,
    orchestrator: tokio::task::JoinHandle<Result<()>>,
    last_cued_event: Option<String>,
    _legacy_events: tokio::task::JoinHandle<()>,
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
        let runtime_events = compat::spawn_runtime_control_bridge(event_rx, bus_tx.clone());
        let mut interaction = cfg.interaction.clone();
        interaction.repeat_debounce_ms = cfg.effective_repeat_debounce_ms();
        let hotkey_handle = hotkey::spawn_listener(interaction, bus_tx)?;

        let signal_handle = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = event_tx.send(compat::RuntimeControlEvent::Quit);
    });

        Ok(Self {
            cfg,
            cue,
            status_rx,
            orchestrator,
            last_cued_event: None,
            _legacy_events: runtime_events,
            _hotkey_handle: hotkey_handle,
            _signal_handle: signal_handle,
        })
    }

    fn publish_status(
        &mut self,
        status: SessionStatus,
        render: StatusRender,
        tray: Option<&mut tray::TrayController>,
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
}

#[cfg(not(target_os = "macos"))]
fn set_tray_status(tray: &mut tray::TrayController, tooltip: String, icon_state: &'static str) {
    tray.set_status(tooltip, icon_state);
}

#[cfg(target_os = "macos")]
fn set_tray_status(_tray: &mut tray::TrayController, tooltip: String, icon_state: &'static str) {
    log::info!("tray status ({icon_state}): {tooltip}");
}

#[cfg(not(target_os = "macos"))]
fn pulse_tray_recording(tray: &mut tray::TrayController) {
    tray.pulse_recording();
}

#[cfg(target_os = "macos")]
fn pulse_tray_recording(_tray: &mut tray::TrayController) {}

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
