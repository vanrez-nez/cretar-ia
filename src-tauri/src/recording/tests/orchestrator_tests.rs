use crate::audio_cues::CuePlayer;
use crate::config::{AppConfig, PipelineConfig, QueueSaturationPolicy, RecoveryStrategyConfig};
use crate::contracts::commands::RecordingCommand;
use crate::contracts::errors::{RecordingErrorCode, RecoveryHint};
use crate::contracts::events::{
    HotkeyEvent, PipelineMode, PipelinePhase, RecordingArtifact, RecordingEvent,
};
use crate::contracts::status::{bounded_status_channel, SessionStatus, SessionStatusReceiver};
use crate::recording::command_bus::CommandBusTx;
use crate::recording::orchestrator;
use tokio::time::{timeout, Duration};

fn harness_config() -> AppConfig {
    let mut cfg = AppConfig::default();
    cfg.pipeline = PipelineConfig {
        debounce_ms: Some(80),
        settle_timeout_ms: Some(120),
        hotkey_queue_capacity: Some(8),
        worker_queue_capacity: Some(8),
        queue_saturation_policy: Some(QueueSaturationPolicy::Retry),
        recovery: Some(RecoveryStrategyConfig {
            retry_start_timeout: true,
            retry_stop_timeout: true,
            retry_processing_timeout: true,
            retry_queue_saturation: false,
        }),
        max_recording_duration_secs: None,
    };
    cfg.interaction.hotkey_queue_capacity = 8;
    cfg.interaction.worker_queue_capacity = 8;
    cfg.audio_cues.enabled = false;
    cfg
}

fn placeholder_artifact() -> RecordingArtifact {
    RecordingArtifact {
        path: "placeholder.wav".into(),
        duration_ms: 0,
    }
}

async fn start_runtime(
    cfg: AppConfig,
) -> (
    CommandBusTx,
    SessionStatusReceiver,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let cue = CuePlayer::new(&cfg.audio_cues, &cfg);
    let (command_tx, status_rx, _audio_level_rx, handle) =
        orchestrator::start_without_workers_for_tests(cfg, cue, None);
    (command_tx, status_rx, handle)
}

async fn shutdown_runtime(
    command_tx: CommandBusTx,
    mut handle: tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let _ = command_tx.send_command(RecordingCommand::Shutdown);
    if timeout(Duration::from_secs(2), &mut handle).await.is_err() {
        handle.abort();
        let _ = handle.await;
        panic!("orchestrator did not shut down within timeout");
    }
}

async fn wait_for_status<F>(status_rx: &mut SessionStatusReceiver, matcher: F) -> SessionStatus
where
    F: Fn(&SessionStatus) -> bool,
{
    loop {
        let status = timeout(Duration::from_secs(5), status_rx.recv())
            .await
            .expect("timed out waiting for orchestrator status")
            .expect("orchestrator status channel closed unexpectedly");
        if matcher(&status) {
            return status;
        }
    }
}

#[tokio::test]
async fn bounded_status_channel_drops_oldest_and_keeps_latest_ordered_state() {
    let (status_tx, mut status_rx) = bounded_status_channel(2);

    let first = SessionStatus::with_defaults(
        PipelinePhase::Idle,
        PipelineMode::PushToTalk,
        7,
        1,
        "first".to_string(),
    );
    let second = SessionStatus::with_defaults(
        PipelinePhase::Starting,
        PipelineMode::PushToTalk,
        7,
        2,
        "second".to_string(),
    );
    let third = SessionStatus::with_defaults(
        PipelinePhase::Recording,
        PipelineMode::PushToTalk,
        7,
        3,
        "third".to_string(),
    );

    assert!(!status_tx.send(first).unwrap().dropped_oldest);
    assert!(!status_tx.send(second).unwrap().dropped_oldest);
    let overflow = status_tx.send(third).unwrap();

    assert!(overflow.dropped_oldest);
    assert_eq!(overflow.dropped_total, 1);

    let kept_second = status_rx.recv().await.expect("second status retained");
    let kept_third = status_rx.recv().await.expect("latest status retained");

    assert_eq!(kept_second.seq, 2);
    assert_eq!(kept_second.source, "second");
    assert_eq!(kept_third.seq, 3);
    assert_eq!(kept_third.state, PipelinePhase::Recording);
    assert_eq!(kept_third.source, "third");
}

#[tokio::test]
async fn cancel_during_starting_moves_to_recovering() {
    let cfg = harness_config();
    let (bus_tx, mut status_rx, handle) = start_runtime(cfg).await;

    let _ = bus_tx.send_hotkey(HotkeyEvent::Pressed);
    if let Some(event) = bus_tx.send_hotkey(HotkeyEvent::CancelPressed) {
        let _ = bus_tx.send_worker(event);
    }

    let recovering = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Recovering
    })
    .await;
    assert_eq!(recovering.source, "cancel_pressed");

    let _ = bus_tx.send_worker(RecordingEvent::RecoveryCompleted);
    let idle = wait_for_status(&mut status_rx, |status| status.state == PipelinePhase::Idle).await;
    assert!(idle.seq > recovering.seq);

    shutdown_runtime(bus_tx, handle).await;
}

#[tokio::test]
async fn cancel_during_recording_moves_to_recovering() {
    let cfg = harness_config();
    let (bus_tx, mut status_rx, handle) = start_runtime(cfg).await;

    let _ = bus_tx.send_hotkey(HotkeyEvent::Pressed);
    let _ = bus_tx.send_worker(RecordingEvent::AudioStarted);
    let _ = bus_tx.send_hotkey(HotkeyEvent::Pressed);

    let _recording = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Recording
    })
    .await;
    let _ = bus_tx.send_hotkey(HotkeyEvent::CancelPressed);
    let recovering = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Recovering
    })
    .await;
    assert_eq!(recovering.error_hint, RecoveryHint::RetryStop);

    let _ = bus_tx.send_worker(RecordingEvent::RecoveryCompleted);
    let idle = wait_for_status(&mut status_rx, |status| status.state == PipelinePhase::Idle).await;
    assert!(idle.seq > recovering.seq);

    shutdown_runtime(bus_tx, handle).await;
}

#[tokio::test]
async fn cancel_during_stopping_moves_to_recovering() {
    let cfg = harness_config();
    let (bus_tx, mut status_rx, handle) = start_runtime(cfg).await;

    let _ = bus_tx.send_hotkey(HotkeyEvent::Pressed);
    let _ = bus_tx.send_worker(RecordingEvent::AudioStarted);
    let _ = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Recording
    })
    .await;

    let _ = bus_tx.send_hotkey(HotkeyEvent::Released);
    let _ = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Stopping
    })
    .await;

    let _ = bus_tx.send_hotkey(HotkeyEvent::CancelPressed);
    let recovering = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Recovering
    })
    .await;
    assert_eq!(recovering.source, "cancel_pressed");

    let _ = bus_tx.send_worker(RecordingEvent::RecoveryCompleted);
    let _ = wait_for_status(&mut status_rx, |status| status.state == PipelinePhase::Idle).await;

    shutdown_runtime(bus_tx, handle).await;
}

#[tokio::test]
async fn cancel_during_processing_moves_to_recovering() {
    let cfg = harness_config();
    let (bus_tx, mut status_rx, handle) = start_runtime(cfg).await;

    let _ = bus_tx.send_hotkey(HotkeyEvent::Pressed);
    let _ = bus_tx.send_worker(RecordingEvent::AudioStarted);
    let _ = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Recording
    })
    .await;

    let _ = bus_tx.send_hotkey(HotkeyEvent::Released);
    let _ = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Stopping
    })
    .await;
    let _ = bus_tx.send_worker(RecordingEvent::AudioStopped {
        artifact: placeholder_artifact(),
    });
    let _ = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Processing
    })
    .await;

    let _ = bus_tx.send_hotkey(HotkeyEvent::CancelPressed);
    let _ = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Recovering
    })
    .await;
    let _ = bus_tx.send_worker(RecordingEvent::RecoveryCompleted);
    let _ = wait_for_status(&mut status_rx, |status| status.state == PipelinePhase::Idle).await;

    shutdown_runtime(bus_tx, handle).await;
}

#[tokio::test]
async fn worker_start_failure_transitions_to_error_then_recoverable() {
    let cfg = harness_config();
    let (bus_tx, mut status_rx, handle) = start_runtime(cfg).await;

    let _ = bus_tx.send_hotkey(HotkeyEvent::Pressed);
    let _ = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Starting
    })
    .await;

    let _ = bus_tx.send_worker(RecordingEvent::AudioStartFailed {
        code: RecordingErrorCode::AudioInit,
        reason: "start failure".into(),
    });
    let error = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Error
    })
    .await;
    assert_eq!(error.error_code, Some(RecordingErrorCode::AudioInit));

    let _ = bus_tx.send_hotkey(HotkeyEvent::CancelPressed);
    let recovering = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Recovering
    })
    .await;
    assert_eq!(recovering.source, "cancel_pressed");

    let _ = bus_tx.send_worker(RecordingEvent::RecoveryCompleted);
    let _ = wait_for_status(&mut status_rx, |status| status.state == PipelinePhase::Idle).await;

    shutdown_runtime(bus_tx, handle).await;
}

#[tokio::test]
async fn worker_stop_failure_transitions_to_error_with_retry_hint() {
    let cfg = harness_config();
    let (bus_tx, mut status_rx, handle) = start_runtime(cfg).await;

    let _ = bus_tx.send_hotkey(HotkeyEvent::Pressed);
    let _ = bus_tx.send_worker(RecordingEvent::AudioStarted);
    let _ = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Recording
    })
    .await;

    let _ = bus_tx.send_hotkey(HotkeyEvent::Released);
    let _ = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Stopping
    })
    .await;
    let _ = bus_tx.send_worker(RecordingEvent::AudioStopFailed {
        code: RecordingErrorCode::AudioStop,
        reason: "stop failure".into(),
    });
    let error = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Error
    })
    .await;
    assert_eq!(error.error_hint, RecoveryHint::RetryStop);
    assert_eq!(error.error_code, Some(RecordingErrorCode::AudioStop));

    shutdown_runtime(bus_tx, handle).await;
}

#[tokio::test]
async fn queue_saturation_under_bursty_hotkey_updates() {
    let mut cfg = harness_config();
    cfg.interaction.hotkey_queue_capacity = 1;
    cfg.pipeline = PipelineConfig {
        hotkey_queue_capacity: Some(1),
        ..cfg.pipeline
    };

    let (bus_tx, mut status_rx, handle) = start_runtime(cfg).await;

    let mut sat_count = 0u32;
    for _ in 0..120 {
        if let Some(event) = bus_tx.send_hotkey(HotkeyEvent::ModeUpdate(PipelineMode::PushToTalk)) {
            let _ = bus_tx.send_worker(event);
            sat_count += 1;
        }
    }

    assert!(sat_count > 0);
    let _ = wait_for_status(&mut status_rx, |status| {
        status.source.starts_with("recording.queue_saturated")
    })
    .await;

    shutdown_runtime(bus_tx, handle).await;
}

#[tokio::test]
async fn process_failure_transitions_to_error_and_recovers() {
    let cfg = harness_config();
    let (bus_tx, mut status_rx, handle) = start_runtime(cfg).await;

    let _ = bus_tx.send_hotkey(HotkeyEvent::Pressed);
    let _ = bus_tx.send_worker(RecordingEvent::AudioStarted);
    let _ = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Recording
    })
    .await;

    let _ = bus_tx.send_hotkey(HotkeyEvent::Released);
    let _ = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Stopping
    })
    .await;
    let _ = bus_tx.send_worker(RecordingEvent::AudioStopped {
        artifact: placeholder_artifact(),
    });
    let _ = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Processing
    })
    .await;

    let _ = bus_tx.send_worker(RecordingEvent::ProcessFailed {
        code: RecordingErrorCode::Processing,
        reason: "worker failure".into(),
    });
    let error = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Error
    })
    .await;
    assert_eq!(error.error_hint, RecoveryHint::RetryProcessing);

    let _ = bus_tx.send_hotkey(HotkeyEvent::CancelPressed);
    let _ = wait_for_status(&mut status_rx, |status| {
        status.state == PipelinePhase::Recovering
    })
    .await;
    let _ = bus_tx.send_worker(RecordingEvent::RecoveryCompleted);
    let _ = wait_for_status(&mut status_rx, |status| status.state == PipelinePhase::Idle).await;

    shutdown_runtime(bus_tx, handle).await;
}
