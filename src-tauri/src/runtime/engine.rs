use crate::config::{AppConfig, TrayConfig};
use crate::contracts::events::PipelinePhase;
use crate::contracts::status::SessionStatus;
use crate::domain::AppEvent;
use crate::openrouter::OpenRouterClient;
use crate::{audio_cues, hotkey, recording, runtime::compat, tray};
use anyhow::{anyhow, Result};
use tokio::sync::mpsc;

#[cfg(not(target_os = "macos"))]
use tokio::time::{interval, Duration, MissedTickBehavior};

#[cfg(not(target_os = "macos"))]
pub fn run_non_macos(cfg: AppConfig) -> Result<()> {
    cfg.validate()?;
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AppEvent>();
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
    runtime.block_on(run_core_non_macos(cfg, &mut tray, event_tx, event_rx))
}

#[cfg(target_os = "macos")]
pub fn run_macos(cfg: AppConfig) -> Result<()> {
    cfg.validate()?;
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AppEvent>();
    let background_cfg = cfg.clone();
    let background_tx = event_tx.clone();
    let config_path = AppConfig::config_path().to_string_lossy().into_owned();
    let cue = audio_cues::CuePlayer::new(&cfg.audio_cues, &cfg);

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

#[cfg(not(target_os = "macos"))]
async fn run_core_non_macos(
    cfg: AppConfig,
    tray: &mut tray::TrayController,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    event_rx: mpsc::UnboundedReceiver<AppEvent>,
) -> Result<()> {
    let openrouter = match OpenRouterClient::new(cfg.provider.openrouter.clone()) {
        Ok(client) => Some(client),
        Err(err) => {
            log::warn!("provider config error: {err}");
            None
        }
    };
    let cue = audio_cues::CuePlayer::new(&cfg.audio_cues, &cfg);
    let (bus_tx, mut status_rx, mut orchestrator) =
        recording::orchestrator::start(cfg.clone(), cue, openrouter);
    let _legacy_events = compat::spawn_legacy_app_event_bridge(event_rx, bus_tx.clone());

    let _hotkey_handle = hotkey::spawn_listener(cfg.interaction.clone(), bus_tx)?;
    let _ = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = event_tx.send(AppEvent::Quit);
    });

    let mut pulse_timer = interval(Duration::from_millis(cfg.tray.refresh_ms.max(1)));
    pulse_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let _ = pulse_timer.tick().await;

    let mut is_recording = false;
    let mut last_cued_event: Option<String> = None;

    loop {
        tokio::select! {
            status = status_rx.recv() => {
        match status {
                    Some(status) => {
                        let render = render_status(&cfg.tray, &status);
                        tray.set_status(render.tooltip, render.icon_state);
                        is_recording = render.should_pulse;

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
                    None => {
                        return match orchestrator.await {
                            Ok(result) => result,
                            Err(err) => Err(anyhow!("orchestrator task stopped unexpectedly: {err}")),
                        };
                    }
                }
            }
            _ = pulse_timer.tick(), if is_recording => {
                tray.pulse_recording();
            }
            result = &mut orchestrator => {
                return result?;
            }
        }
    }
}

#[cfg(target_os = "macos")]
async fn run_core_macos(
    cfg: AppConfig,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    event_rx: mpsc::UnboundedReceiver<AppEvent>,
    cue: audio_cues::CuePlayer,
) -> Result<()> {
    let openrouter = match OpenRouterClient::new(cfg.provider.openrouter.clone()) {
        Ok(client) => Some(client),
        Err(err) => {
            log::warn!("provider config error: {err}");
            None
        }
    };
    let (bus_tx, mut status_rx, mut orchestrator) =
        recording::orchestrator::start(cfg.clone(), cue.clone(), openrouter);
    let _legacy_events = compat::spawn_legacy_app_event_bridge(event_rx, bus_tx.clone());
    let _hotkey_handle = hotkey::spawn_listener(cfg.interaction.clone(), bus_tx)?;

    let _ = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = event_tx.send(AppEvent::Quit);
    });

    let mut last_cued_event: Option<String> = None;

    loop {
        tokio::select! {
            status = status_rx.recv() => {
                if let Some(status) = status {
                    let render = render_status(&cfg.tray, &status);
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
                    log::info!("orchestrator runtime_status: {}", status);
                } else {
                    return match orchestrator.await {
                        Ok(result) => result,
                        Err(err) => Err(anyhow!("orchestrator task stopped unexpectedly: {err}")),
                    };
                }
            }
            result = &mut orchestrator => {
                return result?;
            }
        }
    }
}

#[derive(Clone, Copy)]
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
        cue: audio_cues::status_to_cue(status),
    }
}
