mod audio;
mod audio_cues;
mod config;
mod hotkey;
mod inject;
mod openrouter;
mod tray;
#[cfg(feature = "settings-ui")]
mod settings_ui;

use anyhow::Result;
use config::{AppConfig, AppEvent};
use tokio::sync::mpsc;
use std::time::Instant;
#[cfg(not(target_os = "macos"))]
use tokio::time::{interval, Duration, MissedTickBehavior};

fn is_settings_mode() -> bool {
    std::env::args().any(|arg| arg == "--settings")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppPhase {
    Idle,
    Recording,
    Sending,
}

#[derive(Debug)]
enum WorkEvent {
    Completed,
    Failed(String),
}

pub fn run() -> Result<()> {
    if is_settings_mode() {
        #[cfg(feature = "settings-ui")]
        return settings_ui::run();

        #[cfg(not(feature = "settings-ui"))]
        return Ok(());
    }

    let cfg = AppConfig::load_or_create()?;
    init_logging(&cfg)?;

    log::info!(
        "starting app v{} with interaction {:?} and shortcut {}",
        env!("CARGO_PKG_VERSION"),
        cfg.interaction.mode,
        cfg.interaction.shortcut
    );
    log::info!("config path: {}", AppConfig::config_path().display());

    #[cfg(target_os = "macos")]
    return run_macos(cfg);

    #[cfg(not(target_os = "macos"))]
    return run_non_macos(cfg);
}

fn init_logging(cfg: &AppConfig) -> Result<()> {
    let _ = std::fs::create_dir_all(cfg.base_dir_path().join("logs"));

    let logger = || {
        fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{}][{}][{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                record.level(),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::Debug)
        .chain(std::io::stdout())
    };

    match fern::log_file(config::AppConfig::log_file_path()) {
        Ok(file) => logger().chain(file).apply()?,
        Err(err) => {
            eprintln!("failed to initialize file logger: {err}");
            logger().apply()?;
        }
    }

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn run_non_macos(cfg: AppConfig) -> Result<()> {
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
fn run_macos(cfg: AppConfig) -> Result<()> {
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
    let openrouter = match openrouter::OpenRouterClient::new(cfg.provider.openrouter.clone()) {
        Ok(client) => Some(client),
        Err(err) => {
            log::warn!("provider config invalid: {err}");
            let _ = tray.set_status(format!("provider config error: {err}"), "error");
            cue.play_error();
            None
        }
    };
    let mut recorder: Option<audio::Recorder> = None;
    let mut phase = AppPhase::Idle;

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
                        log::debug!("app event: Start (phase={phase:?})");
                        if phase != AppPhase::Idle {
                            log::debug!("ignoring start while phase={phase:?}");
                            continue;
                        }

                        log::info!("start requested");
                        match audio::Recorder::start(&cfg.audio, cfg.recordings_path()) {
                            Ok(session) => {
                                recorder = Some(session);
                                phase = AppPhase::Recording;
                                log::debug!("state -> Recording");
                                cue.play_start();
                                log::debug!("recording started");
                                tray.set_status(cfg.tray.tooltip.recording.clone(), "recording");
                            }
                            Err(err) => {
                                cue.play_error();
                                log::warn!("start error: {err}");
                                tray.set_status(format!("start error: {err}"), "error");
                            }
                        }
                    }
                    Some(AppEvent::Stop) => {
                        log::debug!("app event: Stop (phase={phase:?})");
                        if phase != AppPhase::Recording {
                            log::debug!("ignoring stop while phase={phase:?}");
                            continue;
                        }

                        phase = AppPhase::Sending;
                        log::debug!("state -> Sending");
                        cue.play_stop();
                        tray.set_status(cfg.tray.tooltip.sending.clone(), "sending");
                        log::info!("stop requested, dispatching transcription");

                        let session = match recorder.take() {
                            Some(session) => session,
                            None => {
                                phase = AppPhase::Idle;
                                tray.set_status("stop error: no active session".to_string(), "error");
                                log::warn!("stop received without active recorder");
                                cue.play_error();
                                continue;
                            }
                        };

                        let stop_started_at = Instant::now();
                        let wav_file = match tokio::task::block_in_place(|| session.stop()) {
                            Ok(path) => path,
                            Err(err) => {
                                phase = AppPhase::Idle;
                                tray.set_status(format!("stop error: {err}"), "error");
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
                        log::debug!("app event: Toggle (phase={phase:?})");
                        match phase {
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
                        tray.set_status("shutting down".to_string(), "shutdown");
                        phase = AppPhase::Idle;
                        drop(hotkey_handle);
                        return Ok(());
                    }
                }
            }
            result = work_rx.recv() => match result {
                Some(WorkEvent::Completed) => {
                    log::info!("transcription pipeline completed");
                    phase = AppPhase::Idle;
                    tray.set_status(cfg.tray.tooltip.success.clone(), "done");
                }
                Some(WorkEvent::Failed(err)) => {
                    log::warn!("transcription pipeline failed: {err}");
                    phase = AppPhase::Idle;
                    tray.set_status(format!("processing error: {err}"), "error");
                    cue.play_error();
                }
                None => {
                    return Ok(());
                }
            },
            _ = pulse_timer.tick(), if phase == AppPhase::Recording => {
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

    let openrouter = match openrouter::OpenRouterClient::new(cfg.provider.openrouter.clone()) {
        Ok(client) => Some(client),
        Err(err) => {
            log::warn!("provider config error: {err}");
            None
        }
    };

    let cue = audio_cues::CuePlayer::new(&cfg.audio_cues, &cfg);
    let mut recorder: Option<audio::Recorder> = None;
    let mut phase = AppPhase::Idle;

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
                        log::debug!("app event: Start (phase={phase:?})");
                        if phase != AppPhase::Idle {
                            log::debug!("ignoring start while phase={phase:?}");
                            continue;
                        }

                        log::info!("start requested");
                        match audio::Recorder::start(&cfg.audio, cfg.recordings_path()) {
                            Ok(session) => {
                                recorder = Some(session);
                                phase = AppPhase::Recording;
                                log::debug!("state -> Recording");
                                cue.play_start();
                                log::debug!("recording started");
                            }
                            Err(err) => {
                                cue.play_error();
                                log::warn!("start error: {err}");
                            }
                        }
                    }
                    Some(AppEvent::Stop) => {
                        log::debug!("app event: Stop (phase={phase:?})");
                        if phase != AppPhase::Recording {
                            log::debug!("ignoring stop while phase={phase:?}");
                            continue;
                        }

                        phase = AppPhase::Sending;
                        log::debug!("state -> Sending");
                        cue.play_stop();
                        log::info!("stop requested, dispatching transcription");

                        let session = match recorder.take() {
                            Some(session) => session,
                            None => {
                                phase = AppPhase::Idle;
                                log::warn!("stop received without active recorder");
                                cue.play_error();
                                continue;
                            }
                        };

                        let stop_started_at = Instant::now();
                        let wav_file = match tokio::task::block_in_place(|| session.stop()) {
                            Ok(path) => path,
                            Err(err) => {
                                phase = AppPhase::Idle;
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
                        log::debug!("app event: Toggle (phase={phase:?})");
                        match phase {
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
                        drop(hotkey_handle);
                        return Ok(());
                    }
                }
            }
            result = work_rx.recv() => match result {
                Some(WorkEvent::Completed) => {
                    log::info!("transcription pipeline completed");
                    phase = AppPhase::Idle;
                }
                Some(WorkEvent::Failed(err)) => {
                    log::warn!("transcription pipeline failed: {err}");
                    phase = AppPhase::Idle;
                    cue.play_error();
                }
                None => return Ok(()),
            }
        }
    }
}

async fn process_recording_work(
    audio_cfg: config::AudioCaptureConfig,
    output_cfg: config::OutputConfig,
    wav_file: std::path::PathBuf,
    openrouter: Option<openrouter::OpenRouterClient>,
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
