use crate::audio;
use crate::audio_cues::CuePlayer;
use crate::config::{AppConfig, QueueSaturationPolicy};
use crate::contracts::commands::RecordingCommand;
use crate::contracts::errors::{RecordingErrorCode, RecoveryHint};
use crate::contracts::events::{HotkeyEvent, PipelinePhase, RecordingEvent};
use crate::contracts::status::{
    bounded_status_channel, SessionStatusReceiver, SessionStatusSender,
    SESSION_STATUS_QUEUE_CAPACITY,
};
use crate::history::HistoryStore;
use crate::media_control::MediaPauseController;
use crate::providers::DynSpeechToTextProvider;
use crate::recording::command_bus::{CommandBus, CommandBusTx};
use crate::recording::fsm::{transition, NoopReason, RecordedEvent, Transition, TransitionResult};
use crate::recording::state::RecordingState;
use crate::recording::telemetry;
use crate::recording::workers::{
    audio_worker::AudioWorker,
    processor_worker::{ProcessorWorker, TransformRuntime},
    recovery::RecoveryWorker,
};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration};

pub fn start(
    cfg: AppConfig,
    cue: CuePlayer,
    stt_provider: Option<DynSpeechToTextProvider>,
) -> (CommandBusTx, SessionStatusReceiver, JoinHandle<Result<()>>) {
    start_with_transform(cfg, cue, stt_provider, TransformRuntime::disabled(), None)
}

pub fn start_with_transform(
    cfg: AppConfig,
    cue: CuePlayer,
    stt_provider: Option<DynSpeechToTextProvider>,
    transform: TransformRuntime,
    history: Option<HistoryStore>,
) -> (CommandBusTx, SessionStatusReceiver, JoinHandle<Result<()>>) {
    start_with_worker_mode(cfg, cue, stt_provider, transform, history, true)
}

#[cfg(test)]
pub fn start_without_workers_for_tests(
    cfg: AppConfig,
    cue: CuePlayer,
    stt_provider: Option<DynSpeechToTextProvider>,
) -> (CommandBusTx, SessionStatusReceiver, JoinHandle<Result<()>>) {
    start_with_worker_mode(
        cfg,
        cue,
        stt_provider,
        TransformRuntime::disabled(),
        None,
        false,
    )
}

fn start_with_worker_mode(
    cfg: AppConfig,
    cue: CuePlayer,
    stt_provider: Option<DynSpeechToTextProvider>,
    transform: TransformRuntime,
    history: Option<HistoryStore>,
    start_workers: bool,
) -> (CommandBusTx, SessionStatusReceiver, JoinHandle<Result<()>>) {
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
            stt_provider,
            transform,
            history,
            cue,
            media_pause: Arc::new(MediaPauseController::new()),
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
            post_stop_started_at: None,
            media_resume_in_flight: None,
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
    stt_provider: Option<DynSpeechToTextProvider>,
    transform: TransformRuntime,
    history: Option<HistoryStore>,
    cue: CuePlayer,
    media_pause: Arc<MediaPauseController>,
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
    post_stop_started_at: Option<Instant>,
    media_resume_in_flight: Option<JoinHandle<()>>,
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
        self.clear_finished_media_resume_task();
        let should_resume_after_audio_stop = matches!(
            &event,
            RecordedEvent::Worker(
                RecordingEvent::AudioStopped { .. } | RecordingEvent::AudioStopFailed { .. }
            )
        );
        let should_resume_after_process_outcome = matches!(
            &event,
            RecordedEvent::Worker(
                RecordingEvent::ProcessCompleted
                    | RecordingEvent::AudioStartFailed { .. }
                    | RecordingEvent::AudioDeviceUnavailable { .. }
                    | RecordingEvent::ProcessFailed { .. }
                    | RecordingEvent::RecoveryFailed { .. }
            )
        );
        let audio_stopped_to_processing_started_at =
            if matches!(
                &event,
                RecordedEvent::Worker(RecordingEvent::AudioStopped { .. })
            ) && matches!(self.state.phase, PipelinePhase::Stopping)
            {
                Some(Instant::now())
            } else {
                None
            };

        if let RecordedEvent::Hotkey(HotkeyEvent::ModeUpdate(mode)) = event {
            self.cfg.interaction.set_mode(mode);
        }

        if let RecordedEvent::Worker(RecordingEvent::TransformFailed { reason }) = &event {
            self.publish_status(
                format!("transform_failed:{reason}"),
                None,
                RecoveryHint::NoRecovery,
            );
            return;
        }

        if let RecordedEvent::Worker(RecordingEvent::AudioStopped { artifact }) = &event {
            if matches!(self.state.phase, PipelinePhase::Stopping) {
                self.pending_recording = Some(artifact.path.clone());
            }
            if let Some(started_at) = self.post_stop_started_at.take() {
                log::info!(
                    "profile.post_stop_audio success=true mode={} stop_to_audio_stopped_event_ms={} artifact_duration_ms={} artifact_path={}",
                    self.state.mode,
                    started_at.elapsed().as_millis(),
                    artifact.duration_ms,
                    artifact.path.display()
                );
            }
        }

        if should_resume_after_audio_stop {
            self.stop_in_flight = false;
            if matches!(
                &event,
                RecordedEvent::Worker(RecordingEvent::AudioStopFailed { .. })
            ) {
                self.cancel_stt_preconnect();
                if let Some(started_at) = self.post_stop_started_at.take() {
                    log::info!(
                        "profile.post_stop_audio success=false mode={} stop_to_audio_stop_failed_event_ms={}",
                        self.state.mode,
                        started_at.elapsed().as_millis()
                    );
                }
            }
        }

        if should_resume_after_process_outcome {
            self.pending_recording = None;
            self.cancel_stt_preconnect();
        }

        let transition = transition(&self.state, event.clone());
        let queues_processing = matches!(
            &transition.result,
            TransitionResult::StateChange {
                command: Some(RecordingCommand::RunProcessing),
                ..
            }
        );
        self.apply_transition(transition, event);
        if queues_processing {
            if let Some(started_at) = audio_stopped_to_processing_started_at {
                log::info!(
                    "profile.post_stop_gap audio_stopped_to_processing_command_ms={}",
                    started_at.elapsed().as_millis()
                );
            }
        }
        if should_resume_after_audio_stop {
            self.resume_media_if_needed_after_recording_stop();
        }
        if should_resume_after_process_outcome {
            self.resume_media_if_needed();
        }
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
                if why == "audio_started" {
                    self.prepare_stt_preconnect();
                }
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
            | RecordedEvent::Worker(RecordingEvent::RecoveryFailed { code, .. }) => Some(*code),
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

    fn prepare_stt_preconnect(&self) {
        if let Some(provider) = &self.stt_provider {
            provider.prepare_transcription_connection();
        }
    }

    fn cancel_stt_preconnect(&self) {
        if let Some(provider) = &self.stt_provider {
            provider.cancel_transcription_connection();
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
            PipelinePhase::Starting
                if self.cfg.recovery_strategy().retry_start_timeout && settle_timeout_ms > 0 =>
            {
                Some((
                    PipelinePhase::Starting,
                    Instant::now() + Duration::from_millis(settle_timeout_ms),
                ))
            }
            PipelinePhase::Stopping
                if self.cfg.recovery_strategy().retry_stop_timeout && settle_timeout_ms > 0 =>
            {
                Some((
                    PipelinePhase::Stopping,
                    Instant::now() + Duration::from_millis(settle_timeout_ms),
                ))
            }
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

        if self.cfg.recording.pause_media {
            self.settling = None;
        }

        if self.cfg.recording.pause_media {
            log::info!("recording start: queueing start cue before media pause");
            self.cue.play_start();
            let effective_input_device = audio::effective_input_device_name(
                config.input_device.as_deref(),
                config.auto_switch_to_primary_device,
            );
            log::debug!(
                "recording start: media route input raw={:?} effective={:?}",
                config.input_device,
                effective_input_device
            );
            let paused = self
                .media_pause
                .pause_for_recording(effective_input_device.as_deref());
            log::info!("recording start: media pause result={paused}");
        } else {
            log::debug!("recording start: media pause disabled");
        }

        if !audio_worker.request_start(config, record_base) {
            self.publish_status(
                "audio_start_command_failed",
                Some(RecordingErrorCode::AudioInit),
                RecoveryHint::RetryStart,
            );
        } else {
            self.adjust_settling();
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
            self.post_stop_started_at = Some(Instant::now());
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
        let stt_provider = self.stt_provider.clone();
        let transform = self.transform.clone();
        let history = self.history.clone();
        let Some(processor_worker) = self.processor_worker.as_ref() else {
            self.publish_status(
                "processing_after_shutdown_ignored",
                Some(RecordingErrorCode::Processing),
                RecoveryHint::Manual,
            );
            return Ok(());
        };

        if !processor_worker.request_run(
            audio_cfg,
            output_cfg,
            recording_path,
            stt_provider,
            transform,
            history,
        ) {
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
        self.cancel_stt_preconnect();
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
        self.cancel_stt_preconnect();
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
        self.cancel_stt_preconnect();
        self.resume_media_if_needed();

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

    fn clear_finished_media_resume_task(&mut self) {
        let finished = self
            .media_resume_in_flight
            .as_ref()
            .map(|task| task.is_finished())
            .unwrap_or(false);
        if finished {
            let _ = self.media_resume_in_flight.take();
        }
    }

    fn resume_media_if_needed(&mut self) {
        if !self.cfg.recording.pause_media {
            return;
        }
        self.clear_finished_media_resume_task();
        if self.media_resume_in_flight.is_some() {
            log::debug!("recording media resume skipped because resume task is already running");
            return;
        }
        let media_pause = Arc::clone(&self.media_pause);
        log::info!("profile.media_resume_now started=true");
        self.media_resume_in_flight = Some(tokio::task::spawn_blocking(move || {
            let started_at = Instant::now();
            let outcome = media_pause.resume_now();
            log::info!(
                "profile.media_resume_now finished=true restored={} resumed={} elapsed_ms={}",
                outcome.restored,
                outcome.resumed,
                started_at.elapsed().as_millis()
            );
        }));
    }

    fn resume_media_if_needed_after_recording_stop(&mut self) {
        if !self.cfg.recording.pause_media {
            return;
        }
        self.clear_finished_media_resume_task();
        if self.media_resume_in_flight.is_some() {
            log::debug!("recording media resume after audio stopped skipped because resume task is already running");
            return;
        }
        let media_pause = Arc::clone(&self.media_pause);
        log::info!("profile.media_resume_after_audio_stopped started=true");
        self.media_resume_in_flight = Some(tokio::task::spawn_blocking(move || {
            let started_at = Instant::now();
            let mut attempts = 1u8;
            let mut outcome = media_pause.resume_after_audio_stopped();
            if !outcome.restored {
                attempts = 2;
                outcome = media_pause.resume_now();
            }
            log::info!(
                "profile.media_resume_after_audio_stopped finished=true restored={} resumed={} attempts={} elapsed_ms={}",
                outcome.restored,
                outcome.resumed,
                attempts,
                started_at.elapsed().as_millis()
            );
        }));
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
