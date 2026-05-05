use crate::config::{AppConfig, AudioCaptureConfig, OutputConfig};
use crate::domain::{
    AppEvent,
    AppPhase,
    RuntimeSessionState,
    WorkEvent,
};
use crate::openrouter::OpenRouterClient;
use crate::{audio, audio_cues, hotkey, inject, tray};
use anyhow::Result;
use std::time::Instant;
use tokio::sync::mpsc;

#[cfg(not(target_os = "macos"))]
use tokio::time::{interval, Duration, MissedTickBehavior};

#[cfg(not(target_os = "macos"))]
pub fn run_non_macos(cfg: AppConfig) -> Result<()> {
    log::debug!("starting non-macos runtime");
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();
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
    runtime.block_on(run_core(cfg, &mut tray, event_tx, &mut event_rx))
}

#[cfg(target_os = "macos")]
pub fn run_macos(cfg: AppConfig) -> Result<()> {
    log::debug!("starting macos background runtime");
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AppEvent>();
    let background_cfg = cfg.clone();
    let background_tx = event_tx.clone();
    let config_path = AppConfig::config_path().to_string_lossy().into_owned();

    let app_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to start macOS background runtime");
        let _ = rt.block_on(run_core_macos(background_cfg, background_tx, event_rx));
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
async fn run_core(
    cfg: AppConfig,
    tray: &mut tray::TrayController,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    event_rx: &mut mpsc::UnboundedReceiver<AppEvent>,
) -> Result<()> {
    let (work_tx, mut work_rx) = mpsc::unbounded_channel::<WorkEvent>();
    let hotkey_handle = hotkey::spawn_listener(cfg.interaction.clone(), event_tx.clone())?;
    let cue = audio_cues::CuePlayer::new(&cfg.audio_cues, &cfg);
    let mut session_state = RuntimeSessionState::new();
    session_state.set_config_ready();

    let openrouter = match OpenRouterClient::new(cfg.provider.openrouter.clone()) {
        Ok(client) => Some(client),
        Err(err) => {
            let err_text = format!("provider config error: {err}");
            log::warn!("{err_text}");
            session_state.fail(err_text.clone());
            tray.set_status(err_text, session_state.status);
            cue.play_error();
            None
        }
    };

    let mut recorder: Option<audio::Recorder> = None;
    let _signal = {
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = event_tx.send(AppEvent::Quit);
        })
    };

    let mut pulse_timer = interval(Duration::from_millis(cfg.tray.refresh_ms.max(1)));
    pulse_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let _ = pulse_timer.tick().await;

    loop {
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Some(AppEvent::Start) => {
                        log::debug!("app event: Start (phase={:?})", session_state.phase);
                        if !session_state.can_start() {
                            log::debug!("ignoring start while phase={:?}", session_state.phase);
                            continue;
                        }

                        log::info!("start requested");
                        match audio::Recorder::start(&cfg.audio, cfg.recordings_path()) {
                            Ok(session) => {
                                recorder = Some(session);
                                session_state.start_recording();
                                log::debug!("state -> Recording");
                                cue.play_start();
                                log::debug!("recording started");
                                tray.set_status(cfg.tray.tooltip.recording.clone(), session_state.status);
                            }
                            Err(err) => {
                                let err_text = format!("start error: {err}");
                                cue.play_error();
                                session_state.fail(&err_text);
                                log::warn!("{err_text}");
                                tray.set_status(err_text, session_state.status);
                            }
                        }
                    }
                    Some(AppEvent::Stop) => {
                        log::debug!("app event: Stop (phase={:?})", session_state.phase);
                        if !session_state.can_stop() {
                            log::debug!("ignoring stop while phase={:?}", session_state.phase);
                            continue;
                        }

                        session_state.stop_recording();
                        log::debug!("state -> Sending");
                        cue.play_stop();
                        tray.set_status(cfg.tray.tooltip.sending.clone(), session_state.status);
                        log::info!("stop requested, dispatching transcription");

                        let session = match recorder.take() {
                            Some(session) => session,
                            None => {
                                let err_text = "stop error: no active session".to_string();
                                session_state.fail(&err_text);
                                tray.set_status(err_text.clone(), session_state.status);
                                log::warn!("stop received without active recorder");
                                cue.play_error();
                                continue;
                            }
                        };

                        let stop_started_at = Instant::now();
                        let wav_file = match tokio::task::block_in_place(|| session.stop()) {
                            Ok(path) => path,
                            Err(err) => {
                                let err_text = format!("stop error: {err}");
                                session_state.fail(&err_text);
                                tray.set_status(err_text.clone(), session_state.status);
                                log::warn!("stop failed: {err}");
                                cue.play_error();
                                continue;
                            }
                        };
                        log::debug!(
                            "recording stopped in {:?} path={:?}",
                            stop_started_at.elapsed(),
                            wav_file
                        );

                        let tx = work_tx.clone();
                        let client = openrouter.clone();
                        let audio_cfg = cfg.audio.clone();
                        let output_cfg = cfg.output.clone();
                        tokio::spawn(async move {
                            let result = process_recording_work(audio_cfg, output_cfg, wav_file, client).await;
                            let _ = tx.send(result);
                        });
                    }
                    Some(AppEvent::Toggle) => {
                        log::debug!("app event: Toggle (phase={:?})", session_state.phase);
                        match session_state.phase {
                            AppPhase::Idle => {
                                log::debug!("toggle triggered -> start");
                                let _ = event_tx.send(AppEvent::Start);
                            }
                            AppPhase::Recording => {
                                log::debug!("toggle triggered -> stop");
                                let _ = event_tx.send(AppEvent::Stop);
                            }
                            AppPhase::Sending => {
                                log::debug!("toggle ignored during sending");
                            }
                        }
                    }
                    Some(AppEvent::Quit) | None => {
                        log::info!("shutdown requested");
                        session_state.request_shutdown();
                        tray.set_status("shutting down".to_string(), session_state.status);
                        drop(hotkey_handle);
                        return Ok(());
                    }
                }
            }
            result = work_rx.recv() => match result {
                Some(WorkEvent::Completed) => {
                    log::info!("transcription pipeline completed");
                    session_state.complete_send();
                    tray.set_status(cfg.tray.tooltip.success.clone(), session_state.status);
                    session_state.reset_to_idle();
                    tray.set_status(cfg.tray.tooltip.idle.clone(), session_state.status);
                }
                Some(WorkEvent::Failed(err)) => {
                    log::warn!("transcription pipeline failed: {err}");
                    let err_text = format!("processing error: {err}");
                    session_state.fail(&err_text);
                    tray.set_status(err_text, session_state.status);
                    cue.play_error();
                }
                None => {
                    session_state.request_shutdown();
                    tray.set_status("shutting down".to_string(), session_state.status);
                    return Ok(());
                }
            },
            _ = pulse_timer.tick(), if session_state.phase == AppPhase::Recording => {
                tray.pulse_recording();
            }
        }
    }
}

#[cfg(target_os = "macos")]
async fn run_core_macos(
    cfg: AppConfig,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    mut event_rx: mpsc::UnboundedReceiver<AppEvent>,
) -> Result<()> {
    let hotkey_handle = hotkey::spawn_listener(cfg.interaction.clone(), event_tx.clone())?;
    let (work_tx, mut work_rx) = mpsc::unbounded_channel::<WorkEvent>();
    log::info!("hotkey listener started");
    let mut session_state = RuntimeSessionState::new();
    session_state.set_config_ready();

    let openrouter = match OpenRouterClient::new(cfg.provider.openrouter.clone()) {
        Ok(client) => Some(client),
        Err(err) => {
            let err_text = format!("provider config error: {err}");
            log::warn!("{}", err_text);
            session_state.fail(err_text);
            None
        }
    };

    let cue = audio_cues::CuePlayer::new(&cfg.audio_cues, &cfg);
    let mut recorder: Option<audio::Recorder> = None;
    let _signal = {
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = event_tx.send(AppEvent::Quit);
        })
    };

    loop {
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Some(AppEvent::Start) => {
                        log::debug!("app event: Start (phase={:?})", session_state.phase);
                        if !session_state.can_start() {
                            log::debug!("ignoring start while phase={:?}", session_state.phase);
                            continue;
                        }

                        log::info!("start requested");
                        match audio::Recorder::start(&cfg.audio, cfg.recordings_path()) {
                            Ok(session) => {
                                recorder = Some(session);
                                session_state.start_recording();
                                log::debug!("state -> Recording");
                                cue.play_start();
                                log::debug!("recording started");
                            }
                            Err(err) => {
                                let err_text = format!("start error: {err}");
                                cue.play_error();
                                session_state.fail(err_text);
                                log::warn!("start error: {err}");
                            }
                        }
                    }
                    Some(AppEvent::Stop) => {
                        log::debug!("app event: Stop (phase={:?})", session_state.phase);
                        if !session_state.can_stop() {
                            log::debug!("ignoring stop while phase={:?}", session_state.phase);
                            continue;
                        }

                        session_state.stop_recording();
                        log::debug!("state -> Sending");
                        cue.play_stop();
                        log::info!("stop requested, dispatching transcription");

                        let session = match recorder.take() {
                            Some(session) => session,
                            None => {
                                let err_text = "stop error: no active session";
                                session_state.fail(err_text);
                                log::warn!("stop received without active recorder");
                                cue.play_error();
                                continue;
                            }
                        };

                        let stop_started_at = Instant::now();
                        let wav_file = match tokio::task::block_in_place(|| session.stop()) {
                            Ok(path) => path,
                            Err(err) => {
                                let err_text = format!("stop error: {err}");
                                session_state.fail(err_text);
                                log::warn!("stop failed: {err}");
                                cue.play_error();
                                continue;
                            }
                        };
                        log::debug!(
                            "recording stopped in {:?} path={:?}",
                            stop_started_at.elapsed(),
                            wav_file
                        );

                        let tx = work_tx.clone();
                        let client = openrouter.clone();
                        let audio_cfg = cfg.audio.clone();
                        let output_cfg = cfg.output.clone();
                        tokio::spawn(async move {
                            let result = process_recording_work(audio_cfg, output_cfg, wav_file, client).await;
                            let _ = tx.send(result);
                        });
                    }
                    Some(AppEvent::Toggle) => {
                        log::debug!("app event: Toggle (phase={:?})", session_state.phase);
                        match session_state.phase {
                            AppPhase::Idle => {
                                log::debug!("toggle triggered -> start");
                                let _ = event_tx.send(AppEvent::Start);
                            }
                            AppPhase::Recording => {
                                log::debug!("toggle triggered -> stop");
                                let _ = event_tx.send(AppEvent::Stop);
                            }
                            AppPhase::Sending => {
                                log::debug!("toggle ignored during sending");
                            }
                        }
                    }
                    Some(AppEvent::Quit) | None => {
                        log::info!("shutdown requested");
                        session_state.request_shutdown();
                        drop(hotkey_handle);
                        return Ok(());
                    }
                }
            }
            result = work_rx.recv() => match result {
                Some(WorkEvent::Completed) => {
                    log::info!("transcription pipeline completed");
                    session_state.complete_send();
                    session_state.reset_to_idle();
                }
                Some(WorkEvent::Failed(err)) => {
                    log::warn!("transcription pipeline failed: {err}");
                    session_state.fail(format!("processing error: {err}"));
                    cue.play_error();
                }
                None => return Ok(()),
            }
        }
    }
}

async fn process_recording_work(
    audio_cfg: AudioCaptureConfig,
    output_cfg: OutputConfig,
    wav_file: std::path::PathBuf,
    openrouter: Option<OpenRouterClient>,
) -> WorkEvent {
    let start = Instant::now();
    log::debug!("processing pipeline for {:?} started", wav_file);
    let result = match openrouter {
        Some(client) => match client.transcribe(&wav_file).await {
            Ok(text) => match inject::deliver_text(&audio_cfg, &output_cfg, &text).await {
                Ok(_) => WorkEvent::Completed,
                Err(err) => WorkEvent::Failed(format!("inject error: {err}")),
            },
            Err(err) => WorkEvent::Failed(format!("transcription error: {err}")),
        },
        None => WorkEvent::Failed("no provider configured".to_string()),
    };
    log::debug!(
        "processing pipeline for {:?} finished in {:?} with {}",
        wav_file,
        start.elapsed(),
        match &result {
            WorkEvent::Completed => "success",
            WorkEvent::Failed(err) => err,
        }
    );

    log::info!("debug mode: keeping recording file for inspection: {wav_file:?}");
    result
}
