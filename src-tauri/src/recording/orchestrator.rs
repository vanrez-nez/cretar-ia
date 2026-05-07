use crate::audio_cues::CuePlayer;
use crate::config::{AppConfig, QueueSaturationPolicy};
use crate::contracts::commands::RecordingCommand;
use crate::contracts::errors::{RecordingErrorCode, RecoveryHint};
use crate::contracts::events::{HotkeyEvent, PipelinePhase, RecordingEvent};
use crate::contracts::status::{
    bounded_status_channel, SessionStatusReceiver, SessionStatusSender, SESSION_STATUS_QUEUE_CAPACITY,
};
use crate::openrouter::OpenRouterClient;
use crate::recording::command_bus::{CommandBus, CommandBusTx};
use crate::recording::fsm::{transition, NoopReason, RecordedEvent, Transition, TransitionResult};
use crate::recording::workers::{
    audio_worker::AudioWorker,
    processor_worker::ProcessorWorker,
    recovery::RecoveryWorker,
};
use crate::recording::state::RecordingState;
use crate::recording::telemetry;
use crate::audio;
use anyhow::Result;
use std::path::PathBuf;
use std::time::Instant;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration};

pub fn start(
    cfg: AppConfig,
    _cue: CuePlayer,
    openrouter: Option<OpenRouterClient>,
) -> (
    CommandBusTx,
    SessionStatusReceiver,
    JoinHandle<Result<()>>,
) {
    start_with_worker_mode(cfg, openrouter, true)
}

#[cfg(test)]
pub fn start_without_workers_for_tests(
    cfg: AppConfig,
    _cue: CuePlayer,
    openrouter: Option<OpenRouterClient>,
) -> (
    CommandBusTx,
    SessionStatusReceiver,
    JoinHandle<Result<()>>,
) {
    start_with_worker_mode(cfg, openrouter, false)
}

fn start_with_worker_mode(
    cfg: AppConfig,
    openrouter: Option<OpenRouterClient>,
    start_workers: bool,
) -> (
    CommandBusTx,
    SessionStatusReceiver,
    JoinHandle<Result<()>>,
) {
    let bus = CommandBus::new(&cfg);
    let tx = bus.sender();
    let (status_tx, status_rx) = bounded_status_channel(SESSION_STATUS_QUEUE_CAPACITY);
    let runner_tx = tx.clone();
    let initial_mode = cfg.interaction.pipeline_mode();

    let worker_set = if start_workers {
        let audio_worker = AudioWorker::start(runner_tx.clone());
        let processor_worker = ProcessorWorker::start(runner_tx.clone());
        let recovery_worker = RecoveryWorker::start(
            runner_tx.clone(),
            audio_worker.handle(),
            processor_worker.handle(),
        );
        Some((audio_worker, processor_worker, recovery_worker))
    } else {
        None
    };

    let (audio_worker, processor_worker, recovery_worker) = match worker_set {
        Some((audio_worker, processor_worker, recovery_worker)) => (
            Some(audio_worker),
            Some(processor_worker),
            Some(recovery_worker),
        ),
        None => (None, None, None),
    };

    let handle = tokio::spawn(async move {
        let mut runner = Orchestrator {
            cfg,
            openrouter,
            bus,
            tx: runner_tx,
            audio_worker,
            processor_worker,
            recovery_worker,
            status_tx,
            state: RecordingState::new(initial_mode, 0),
            pending_recording: None,
            settling: None,
            stop_in_flight: false,
            shutdown_started: false,
            max_recording_limit_triggered: false,
            last_device_recovery_check: Instant::now(),
            status_seq: 0,
            phase_started_at: Instant::now(),
        };
        runner.run().await
    });

    (tx, status_rx, handle)
}

struct Orchestrator {
    cfg: AppConfig,
    openrouter: Option<OpenRouterClient>,
    bus: CommandBus,
    tx: CommandBusTx,
    audio_worker: Option<AudioWorker>,
    processor_worker: Option<ProcessorWorker>,
    recovery_worker: Option<RecoveryWorker>,
    status_tx: SessionStatusSender,
    state: RecordingState,
    pending_recording: Option<PathBuf>,
    settling: Option<(PipelinePhase, Instant)>,
    stop_in_flight: bool,
    shutdown_started: bool,
    max_recording_limit_triggered: bool,
    last_device_recovery_check: Instant,
    status_seq: u64,
    phase_started_at: Instant,
}

impl Orchestrator {
    async fn run(&mut self) -> Result<()> {
        let mut settle_check = time::interval(Duration::from_millis(40));
        settle_check.tick().await;

        self.publish_status("runtime_started", None, RecoveryHint::NoRecovery);

        loop {
            tokio::select! {
                Some(event) = self.bus.hotkey_rx.recv() => {
                    self.on_event(RecordedEvent::Hotkey(event));
                }
                Some(event) = self.bus.worker_rx.recv() => {
                    self.on_event(RecordedEvent::Worker(event));
                }
                Some(command) = self.bus.command_rx.recv() => {
                    if !self.on_command(command).await? {
                        return Ok(());
                    }
                }
                _ = settle_check.tick() => {
                    self.check_settling_timeout();
                    self.check_audio_device_recovery();
                    self.check_max_recording_duration().await?;
                }
            }
        }
    }

    fn on_event(&mut self, event: RecordedEvent) {
        if let RecordedEvent::Hotkey(HotkeyEvent::ModeUpdate(mode)) = event {
            self.cfg.interaction.set_mode(mode);
        }

        if let RecordedEvent::Worker(RecordingEvent::AudioStopped { path }) = &event {
            if matches!(self.state.phase, PipelinePhase::Stopping) {
                self.pending_recording = Some(path.clone());
            }
        }

        if matches!(
            &event,
            RecordedEvent::Worker(
                RecordingEvent::AudioStopped { .. } | RecordingEvent::AudioStopFailed { .. }
            )
        ) {
            self.stop_in_flight = false;
        }

        if matches!(
            &event,
            RecordedEvent::Worker(
                RecordingEvent::ProcessCompleted
                    | RecordingEvent::AudioStartFailed { .. }
                    | RecordingEvent::AudioDeviceUnavailable { .. }
            )
        ) {
            self.pending_recording = None;
        }

        let transition = transition(&self.state, event.clone());
        self.apply_transition(transition, event);
    }

    async fn on_command(&mut self, command: RecordingCommand) -> Result<bool> {
        match command {
            RecordingCommand::StartRecording => {
                self.handle_start().await?;
            }
            RecordingCommand::StopRecording => {
                self.handle_stop().await?;
            }
            RecordingCommand::RunProcessing => {
                self.handle_run_processing().await?;
            }
            RecordingCommand::CancelProcessing => {
                self.handle_cancel_processing();
            }
            RecordingCommand::ForceStop => {
                self.handle_force_stop().await?;
            }
            RecordingCommand::Shutdown => {
                self.handle_shutdown().await?;
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn apply_transition(&mut self, transition: Transition, triggering_event: RecordedEvent) {
        match transition.result {
            TransitionResult::StateChange {
                from: _,
                to: _,
                why,
                command,
            } => {
                let prior_phase = self.state.phase;
                self.state = transition.next;
                if self.state.phase != prior_phase {
                    self.phase_started_at = Instant::now();
                }
                self.publish_status(
                    why,
                    self.error_code_for_transition_event(&triggering_event),
                    self.state.recovery_hint,
                );
                if let Some(command) = command {
                    self.enqueue_command(command);
                }
            }
            TransitionResult::Noop(NoopReason::QueueSaturated { source, dropped }) => {
                self.publish_queue_saturation(&source, dropped);
            }
            TransitionResult::Noop(_) => {}
        }
    }

    fn error_code_for_transition_event(&self, event: &RecordedEvent) -> Option<RecordingErrorCode> {
        match event {
            RecordedEvent::Worker(RecordingEvent::AudioStartFailed { code, .. })
            | RecordedEvent::Worker(RecordingEvent::AudioStopFailed { code, .. })
            | RecordedEvent::Worker(RecordingEvent::AudioDeviceUnavailable { code, .. })
            | RecordedEvent::Worker(RecordingEvent::ProcessFailed { code, .. })
            | RecordedEvent::Worker(RecordingEvent::RecoveryFailed { code, .. }) => {
                Some(*code)
            }
            RecordedEvent::Worker(RecordingEvent::QueueSaturated { .. }) => {
                Some(RecordingErrorCode::QueueOverflow)
            }
            _ => None,
        }
    }

    fn enqueue_command(&mut self, command: RecordingCommand) {
        if let Some(event) = self.tx.send_command(command) {
            if let Some(saturated) = self.tx.send_worker(event) {
                self.publish_queue_event(&saturated);
            }
        }
    }

    fn publish_queue_event(&mut self, event: &RecordingEvent) {
        if let RecordingEvent::QueueSaturated { source, dropped } = event {
            self.publish_queue_saturation(source, *dropped);
        }
    }

    fn publish_queue_saturation(&mut self, source: &str, dropped: u32) {
        let hint = if self.cfg.queue_saturation_policy() == QueueSaturationPolicy::ErrorOnly
            || !self.cfg.recovery_strategy().retry_queue_saturation
        {
            RecoveryHint::Manual
        } else {
            RecoveryHint::RetryWorker
        };
        self.publish_status(
            format!("recording.queue_saturated:{source}:{dropped}"),
            Some(RecordingErrorCode::QueueOverflow),
            hint,
        );
    }

    fn adjust_settling(&mut self) {
        let settle_timeout_ms = self.cfg.effective_settle_timeout_ms();
        self.settling = match self.state.phase {
            PipelinePhase::Starting if self.cfg.recovery_strategy().retry_start_timeout && settle_timeout_ms > 0 => Some((
                PipelinePhase::Starting,
                Instant::now() + Duration::from_millis(settle_timeout_ms),
            )),
            PipelinePhase::Stopping if self.cfg.recovery_strategy().retry_stop_timeout && settle_timeout_ms > 0 => Some((
                PipelinePhase::Stopping,
                Instant::now() + Duration::from_millis(settle_timeout_ms),
            )),
            _ => None,
        };
    }

    fn check_settling_timeout(&mut self) {
        let Some((phase, deadline)) = self.settling else {
            return;
        };

        if Instant::now() >= deadline {
            self.settling = None;
            let should_transition = match phase {
                PipelinePhase::Starting => self.cfg.recovery_strategy().retry_start_timeout,
                PipelinePhase::Stopping => self.cfg.recovery_strategy().retry_stop_timeout,
                PipelinePhase::Processing => self.cfg.recovery_strategy().retry_processing_timeout,
                _ => false,
            };

            if should_transition {
                self.on_event(RecordedEvent::Worker(RecordingEvent::TimeoutExpired));
            } else {
                self.publish_status(
                    format!("{phase:?}_timeout_no_recovery"),
                    Some(RecordingErrorCode::WorkerTimeout),
                    RecoveryHint::Manual,
                );
            }
        }
    }

    fn check_audio_device_recovery(&mut self) {
        if self.state.phase != PipelinePhase::Error {
            return;
        }
        if self.last_device_recovery_check.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_device_recovery_check = Instant::now();

        let Some(reason) = self.state.last_reason.as_deref() else {
            return;
        };
        if !is_audio_device_unavailable_reason(reason) {
            return;
        }
        if !audio::input_device_ready(&self.cfg.audio) {
            return;
        }

        self.on_event(RecordedEvent::Worker(RecordingEvent::RecoveryCompleted));
    }

    async fn check_max_recording_duration(&mut self) -> Result<()> {
        let Some(max_secs) = self.cfg.effective_max_recording_duration_secs() else {
            return Ok(());
        };

        if self.state.phase != PipelinePhase::Recording {
            self.max_recording_limit_triggered = false;
            return Ok(());
        }

        if self.max_recording_limit_triggered {
            return Ok(());
        }

        if self.phase_started_at.elapsed() >= Duration::from_secs(max_secs) {
            self.max_recording_limit_triggered = true;
            self.publish_status(
                "recording_duration_limit_reached",
                Some(RecordingErrorCode::WorkerTimeout),
                RecoveryHint::RetryStop,
            );
            self.handle_stop().await?;
        }

        Ok(())
    }

    async fn handle_start(&mut self) -> Result<()> {
        let config = self.cfg.audio.clone();
        let record_base = self.cfg.recordings_path();
        let Some(audio_worker) = self.audio_worker.as_ref() else {
            self.publish_status(
                "audio_start_after_shutdown_ignored",
                Some(RecordingErrorCode::AudioInit),
                RecoveryHint::Manual,
            );
            return Ok(());
        };

        if !audio_worker.request_start(config, record_base) {
            self.publish_status(
                "audio_start_command_failed",
                Some(RecordingErrorCode::AudioInit),
                RecoveryHint::RetryStart,
            );
        }
        Ok(())
    }

    async fn handle_stop(&mut self) -> Result<()> {
        if self.stop_in_flight {
            return Ok(());
        }

        let Some(audio_worker) = self.audio_worker.as_ref() else {
            self.publish_status(
                "audio_stop_after_shutdown_ignored",
                Some(RecordingErrorCode::AudioStop),
                RecoveryHint::Manual,
            );
            return Ok(());
        };

        if !audio_worker.request_stop() {
            self.publish_status(
                "audio_stop_command_failed",
                Some(RecordingErrorCode::AudioStop),
                RecoveryHint::RetryStop,
            );
        } else {
            self.stop_in_flight = true;
        }
        Ok(())
    }

    async fn handle_run_processing(&mut self) -> Result<()> {
        let Some(recording_path) = self.pending_recording.clone() else {
            let event = RecordingEvent::ProcessFailed {
                code: RecordingErrorCode::Processing,
                reason: "no recorded audio available".to_string(),
            };
            if self.tx.send_worker(event.clone()).is_some() {
                self.publish_queue_event(&event);
            }
            return Ok(());
        };

        let audio_cfg = self.cfg.audio.clone();
        let output_cfg = self.cfg.output.clone();
        let openrouter = self.openrouter.clone();
        let Some(processor_worker) = self.processor_worker.as_ref() else {
            self.publish_status(
                "processing_after_shutdown_ignored",
                Some(RecordingErrorCode::Processing),
                RecoveryHint::Manual,
            );
            return Ok(());
        };

        if !processor_worker.request_run(audio_cfg, output_cfg, recording_path, openrouter) {
            self.publish_status(
                "processing_start_command_failed",
                Some(RecordingErrorCode::Processing),
                RecoveryHint::RetryProcessing,
            );
        }

        Ok(())
    }

    fn handle_cancel_processing(&mut self) {
        self.pending_recording = None;
        let Some(processor_worker) = self.processor_worker.as_ref() else {
            return;
        };

        if !processor_worker.request_cancel() {
            let event = RecordingEvent::RecoveryFailed {
                code: RecordingErrorCode::Processing,
                reason: "failed to cancel processing".to_string(),
            };
            if self.tx.send_worker(event.clone()).is_some() {
                self.publish_queue_event(&event);
            }
            return;
        }

        let Some(recovery_worker) = self.recovery_worker.as_ref() else {
            return;
        };

        if !recovery_worker.request_recovery() {
            let event = RecordingEvent::RecoveryFailed {
                code: RecordingErrorCode::Unknown,
                reason: "failed to start recovery".to_string(),
            };
            if self.tx.send_worker(event.clone()).is_some() {
                self.publish_queue_event(&event);
            }
        }
    }

    async fn handle_force_stop(&mut self) -> Result<()> {
        let Some(audio_worker) = self.audio_worker.as_ref() else {
            self.publish_status(
                "force_stop_after_shutdown_ignored",
                Some(RecordingErrorCode::AudioStop),
                RecoveryHint::Manual,
            );
            return Ok(());
        };

        if !audio_worker.request_force_stop() {
            self.publish_status(
                "force_stop_command_failed",
                Some(RecordingErrorCode::AudioStop),
                RecoveryHint::RetryWorker,
            );
            if self.state.phase == PipelinePhase::Recovering {
                let event = RecordingEvent::RecoveryFailed {
                    code: RecordingErrorCode::Unknown,
                    reason: "failed to request force stop".to_string(),
                };
                if self.tx.send_worker(event.clone()).is_some() {
                    self.publish_queue_event(&event);
                }
            }
            return Ok(());
        } else {
            self.stop_in_flight = true;
        }

        if self.state.phase == PipelinePhase::Recovering {
            let Some(recovery_worker) = self.recovery_worker.as_ref() else {
                return Ok(());
            };

            if !recovery_worker.request_recovery() {
                let event = RecordingEvent::RecoveryFailed {
                    code: RecordingErrorCode::Unknown,
                    reason: "failed to start recovery".to_string(),
                };
                if self.tx.send_worker(event.clone()).is_some() {
                    self.publish_queue_event(&event);
                }
            }
        }
        Ok(())
    }

    async fn handle_shutdown(&mut self) -> Result<()> {
        if self.shutdown_started {
            return Ok(());
        }
        self.shutdown_started = true;

        let drained = self.bus.close_and_drain();
        log::debug!(
            "orchestrator command bus closed for shutdown; drained hotkey={} worker={} command={}",
            drained.hotkey,
            drained.worker,
            drained.command
        );

        if let Some(audio_worker) = self.audio_worker.take() {
            audio_worker.shutdown().await;
        }
        if let Some(processor_worker) = self.processor_worker.take() {
            processor_worker.shutdown().await;
        }
        if let Some(recovery_worker) = self.recovery_worker.take() {
            recovery_worker.shutdown().await;
        }
        Ok(())
    }

    fn publish_status(
        &mut self,
        last_event: impl Into<String>,
        error_code: Option<RecordingErrorCode>,
        error_hint: RecoveryHint,
    ) {
        self.adjust_settling();
        self.status_seq = self.status_seq.saturating_add(1);
        let status = telemetry::status(
            &self.state,
            self.status_seq,
            self.phase_started_at,
            error_code,
            error_hint,
            last_event,
        );
        match self.status_tx.send(status) {
            Ok(outcome) if outcome.dropped_oldest => {
                log::warn!(
                    "orchestrator telemetry channel full; dropped oldest status (total_dropped={})",
                    outcome.dropped_total
                );
            }
            Ok(_) => {}
            Err(err) => {
                log::warn!("orchestrator telemetry channel closed: {err:?}");
            }
        }
    }
}

fn is_audio_device_unavailable_reason(reason: &str) -> bool {
    reason.contains("configured audio input device")
        || reason.contains("no default input device found")
        || reason.contains("device is no longer available")
        || reason.contains("audio stream error")
}
