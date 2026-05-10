use crate::config::{AudioCaptureConfig, OutputConfig};
use crate::contracts::errors::RecordingErrorCode;
use crate::contracts::events::RecordingEvent;
use crate::history::{HistoryEntry, HistoryStore};
use crate::inject;
use crate::model_health::ModelHealthStatus;
use crate::prompts::PromptView;
use crate::providers::{DynFormattingProvider, DynSpeechToTextProvider};
use crate::recording::command_bus::CommandBusTx;
use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant as StdInstant;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;

const PROCESSOR_WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const MIN_TRANSCRIPTION_AUDIO_DURATION_MS: u64 = 1_000;

#[derive(Clone)]
pub struct TransformRuntime {
    enabled: bool,
    formatter: Option<DynFormattingProvider>,
    prompt: Option<PromptView>,
    health: Option<ModelHealthStatus>,
}

impl TransformRuntime {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            formatter: None,
            prompt: None,
            health: None,
        }
    }

    pub fn new(
        enabled: bool,
        formatter: Option<DynFormattingProvider>,
        prompt: Option<PromptView>,
        health: Option<ModelHealthStatus>,
    ) -> Self {
        Self {
            enabled,
            formatter,
            prompt,
            health,
        }
    }
}

enum ProcessorWorkerCommand {
    Run {
        audio_cfg: AudioCaptureConfig,
        output_cfg: OutputConfig,
        wav_file: PathBuf,
        stt_provider: Option<DynSpeechToTextProvider>,
        transform: TransformRuntime,
        history: Option<HistoryStore>,
    },
    Cancel,
    Shutdown {
        ack: oneshot::Sender<()>,
    },
}

#[derive(Clone)]
pub struct ProcessorWorkerHandle {
    command_tx: UnboundedSender<ProcessorWorkerCommand>,
}

impl ProcessorWorkerHandle {
    pub fn request_cancel(&self) -> bool {
        self.command_tx.send(ProcessorWorkerCommand::Cancel).is_ok()
    }
}

#[derive(Debug)]
pub struct ProcessorWorker {
    command_tx: UnboundedSender<ProcessorWorkerCommand>,
    handle: JoinHandle<()>,
}

impl ProcessorWorker {
    pub fn start(bus_tx: CommandBusTx) -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let tx = bus_tx.clone();

        let handle = tokio::spawn(async move {
            worker_loop(command_rx, tx).await;
        });

        Self { command_tx, handle }
    }

    pub fn handle(&self) -> ProcessorWorkerHandle {
        ProcessorWorkerHandle {
            command_tx: self.command_tx.clone(),
        }
    }

    pub fn request_run(
        &self,
        audio_cfg: AudioCaptureConfig,
        output_cfg: OutputConfig,
        wav_file: PathBuf,
        stt_provider: Option<DynSpeechToTextProvider>,
        transform: TransformRuntime,
        history: Option<HistoryStore>,
    ) -> bool {
        self.command_tx
            .send(ProcessorWorkerCommand::Run {
                audio_cfg,
                output_cfg,
                wav_file,
                stt_provider,
                transform,
                history,
            })
            .is_ok()
    }

    pub fn request_cancel(&self) -> bool {
        self.command_tx.send(ProcessorWorkerCommand::Cancel).is_ok()
    }

    pub async fn shutdown(self) {
        let (ack_tx, ack_rx) = oneshot::channel();
        if self
            .command_tx
            .send(ProcessorWorkerCommand::Shutdown { ack: ack_tx })
            .is_ok()
        {
            let _ = timeout(PROCESSOR_WORKER_SHUTDOWN_TIMEOUT, ack_rx).await;
        }
        let _ = timeout(PROCESSOR_WORKER_SHUTDOWN_TIMEOUT, self.handle).await;
    }
}

async fn worker_loop(mut command_rx: UnboundedReceiver<ProcessorWorkerCommand>, tx: CommandBusTx) {
    let (result_tx, mut result_rx) =
        mpsc::unbounded_channel::<Result<(), (RecordingErrorCode, String)>>();
    let mut processing_task: Option<JoinHandle<()>> = None;

    loop {
        tokio::select! {
            Some(command) = command_rx.recv() => {
                match command {
                    ProcessorWorkerCommand::Run {
                        audio_cfg,
                        output_cfg,
                        wav_file,
                        stt_provider,
                        transform,
                        history,
                    } => {
                        if processing_task.is_some() {
                            if tx.send_worker(RecordingEvent::ProcessFailed {
                                code: RecordingErrorCode::Processing,
                                reason: "processing already running".to_string(),
                            }).is_some()
                            {
                                log::warn!("processing failure event dropped because worker queue was full");
                            }
                            continue;
                        }

                        if tx.send_worker(RecordingEvent::ProcessStarted).is_some() {
                            log::warn!("processing started event dropped because worker queue was full");
                        }

                        let start = StdInstant::now();
                        let result_tx = result_tx.clone();
                        processing_task = Some(tokio::spawn(async move {
                            let result =
                                process_recording_work(
                                    audio_cfg,
                                    output_cfg,
                                    wav_file,
                                    stt_provider,
                                    transform,
                                    history,
                                )
                                .await;
                            let success = result.is_ok();
                            if let Err((code, reason)) = &result {
                                log::warn!(
                                    "processing task failed in {:?}: code={} reason={}",
                                    start.elapsed(),
                                    code,
                                    reason
                                );
                            }
                            let _ = result_tx.send(result);
                            log::debug!(
                                "processing task completed in {:?}, success={}",
                                start.elapsed(),
                                success
                            );
                        }));
                    }
                    ProcessorWorkerCommand::Cancel => {
                        if let Some(task) = processing_task.take() {
                            task.abort();
                        }
                    }
                    ProcessorWorkerCommand::Shutdown { ack } => {
                        if let Some(task) = processing_task.take() {
                            task.abort();
                        }
                        let _ = ack.send(());
                        break;
                    }
                }
            }
            Some(result) = result_rx.recv() => {
                processing_task = None;
                let outcome = match result {
                    Ok(()) => RecordingEvent::ProcessCompleted,
                    Err((code, reason)) => {
                        log::warn!("processing outcome failed: code={} reason={}", code, reason);
                        RecordingEvent::ProcessFailed { code, reason }
                    }
                };

                if tx.send_worker(outcome).is_some() {
                    log::warn!("processing outcome event dropped because worker queue was full");
                }
            }
            else => {
                break;
            }
        }
    }
}

async fn process_recording_work(
    audio_cfg: AudioCaptureConfig,
    output_cfg: OutputConfig,
    wav_file: PathBuf,
    stt_provider: Option<DynSpeechToTextProvider>,
    transform: TransformRuntime,
    history: Option<HistoryStore>,
) -> Result<(), (RecordingErrorCode, String)> {
    if wav_file.as_os_str().is_empty() {
        return Err((
            RecordingErrorCode::Processing,
            "missing recording path".to_string(),
        ));
    }
    let audio_duration_ms = wav_duration_ms(&wav_file).unwrap_or(0);

    let mut record = ProcessRecord::empty();
    let result = if audio_duration_ms <= MIN_TRANSCRIPTION_AUDIO_DURATION_MS {
        log::warn!(
            "recording too short for transcription: duration_ms={} min_duration_ms={}",
            audio_duration_ms,
            MIN_TRANSCRIPTION_AUDIO_DURATION_MS
        );
        let reason = friendly_recording_too_short_error();
        record.error_message = Some(reason.clone());
        Err((RecordingErrorCode::Processing, reason))
    } else {
        match run_transcript_step(stt_provider, &wav_file, output_cfg.processing_timeout_ms).await {
            Ok(transcript_text) => {
                record.transcript_text = Some(transcript_text.clone());
                let mut output_text = transcript_text.clone();

                match run_transform_step(
                    &transcript_text,
                    transform,
                    output_cfg.processing_timeout_ms,
                )
                .await
                {
                    TransformAttempt::Skipped => {}
                    TransformAttempt::Succeeded(text) => {
                        output_text = text.clone();
                        record.transform_text = Some(text);
                    }
                    TransformAttempt::Failed { friendly_message } => {
                        record.error_message = Some(friendly_message);
                    }
                }

                match inject::deliver_text(&audio_cfg, &output_cfg, &output_text).await {
                    Ok(_) => Ok(()),
                    Err(err) => {
                        log::warn!("processing inject failed: {err:?}");
                        let reason = friendly_inject_error();
                        record.error_message = Some(reason.clone());
                        Err((RecordingErrorCode::Processing, reason))
                    }
                }
            }
            Err((code, reason)) => {
                record.error_message = Some(reason.clone());
                Err((code, reason))
            }
        }
    };

    if output_cfg.cleanup_recording_after_processing {
        if !wav_file.exists() {
            log::trace!("audio artifact already removed: {:?}", wav_file);
        } else if let Err(err) = std::fs::remove_file(&wav_file) {
            log::warn!("failed to cleanup audio artifact {:?}: {:?}", wav_file, err);
        } else {
            log::debug!("removed audio artifact {:?}", wav_file);
        }
    } else if wav_file.exists() {
        log::trace!("audio artifact retained by policy: {:?}", wav_file);
    }

    if let Some(history) = history {
        if let Err(err) = history
            .insert(HistoryEntry {
                audio_file_path: available_audio_path(&wav_file),
                audio_duration_ms,
                transcript_text: record.transcript_text,
                transform_text: record.transform_text,
                error_message: record.error_message,
            })
            .await
        {
            log::warn!("failed to write history row: {err:#}");
        }
    }

    let result = result.map(|_| ());

    result
}

#[derive(Default)]
struct ProcessRecord {
    transcript_text: Option<String>,
    transform_text: Option<String>,
    error_message: Option<String>,
}

impl ProcessRecord {
    fn empty() -> Self {
        Self::default()
    }
}

enum TransformAttempt {
    Skipped,
    Succeeded(String),
    Failed { friendly_message: String },
}

enum StepJoinError {
    Timeout,
    Join(tokio::task::JoinError),
}

async fn run_transcript_step(
    stt_provider: Option<DynSpeechToTextProvider>,
    wav_file: &PathBuf,
    timeout_ms: u64,
) -> Result<String, (RecordingErrorCode, String)> {
    let Some(client) = stt_provider else {
        return Err((
            RecordingErrorCode::Processing,
            "Choose a transcript model before recording.".to_string(),
        ));
    };

    let wav_file = wav_file.clone();
    let task = tokio::spawn(async move { client.transcribe(&wav_file).await });

    match join_step_task(task, timeout_ms).await {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(err)) => {
            log::warn!("processing transcription failed: {err:?}");
            Err((
                RecordingErrorCode::Processing,
                friendly_transcription_error(),
            ))
        }
        Err(StepJoinError::Timeout) => {
            log::warn!("speech-to-text step timed out after {timeout_ms}ms");
            Err((RecordingErrorCode::WorkerTimeout, friendly_timeout_error()))
        }
        Err(StepJoinError::Join(err)) => {
            log::warn!("speech-to-text step crashed: {err:?}");
            Err((
                RecordingErrorCode::Processing,
                friendly_transcription_error(),
            ))
        }
    }
}

async fn run_transform_step(
    transcript: &str,
    transform: TransformRuntime,
    timeout_ms: u64,
) -> TransformAttempt {
    let transcript = transcript.to_string();
    let task = tokio::spawn(async move { run_transform_step_inner(transcript, transform).await });

    match join_step_task(task, timeout_ms).await {
        Ok(attempt) => attempt,
        Err(StepJoinError::Timeout) => {
            log::warn!("transform step timed out after {timeout_ms}ms; falling back to transcript");
            TransformAttempt::Failed {
                friendly_message: friendly_transform_timeout_error(),
            }
        }
        Err(StepJoinError::Join(err)) => {
            log::warn!("transform step crashed; falling back to transcript: {err:?}");
            TransformAttempt::Failed {
                friendly_message: friendly_transform_error(),
            }
        }
    }
}

async fn join_step_task<T>(
    mut task: JoinHandle<T>,
    timeout_ms: u64,
) -> std::result::Result<T, StepJoinError> {
    if timeout_ms == 0 {
        return task.await.map_err(StepJoinError::Join);
    }

    match timeout(Duration::from_millis(timeout_ms), &mut task).await {
        Ok(result) => result.map_err(StepJoinError::Join),
        Err(_) => {
            task.abort();
            Err(StepJoinError::Timeout)
        }
    }
}

async fn run_transform_step_inner(
    transcript: String,
    transform: TransformRuntime,
) -> TransformAttempt {
    if !transform.enabled {
        return TransformAttempt::Skipped;
    }

    let Some(formatter) = transform.formatter else {
        log::warn!("transform skipped: no formatting model configured");
        return TransformAttempt::Skipped;
    };

    let Some(prompt) = transform.prompt else {
        log::warn!("transform skipped: no active prompt configured");
        return TransformAttempt::Skipped;
    };

    if matches!(transform.health, Some(ModelHealthStatus::Unhealthy)) {
        let reason =
            "The selected transform model is unavailable, so the original transcript was used."
                .to_string();
        log::warn!("transform failed: {reason}");
        return TransformAttempt::Failed {
            friendly_message: reason,
        };
    }

    log::info!(
        "transform request started prompt_id={} prompt_name={} transcript_len={} health={:?}",
        prompt.id,
        prompt.name,
        transcript.len(),
        transform.health.unwrap_or(ModelHealthStatus::Unknown)
    );

    match formatter.format(&transcript, &prompt.template).await {
        Ok(text) => {
            let text = text.trim().to_string();
            if text.is_empty() {
                let reason = "Transform returned empty text, so the original transcript was used."
                    .to_string();
                log::warn!("transform failed: {reason}");
                TransformAttempt::Failed {
                    friendly_message: reason,
                }
            } else {
                log::info!(
                    "transform completed prompt_id={} transcript_len={} transformed_len={}",
                    prompt.id,
                    transcript.len(),
                    text.len()
                );
                TransformAttempt::Succeeded(text)
            }
        }
        Err(err) => {
            let reason = "Transform failed, so the original transcript was used.".to_string();
            log::warn!("transform failed: {reason} Details: {err:#}");
            TransformAttempt::Failed {
                friendly_message: reason,
            }
        }
    }
}

fn wav_duration_ms(path: &PathBuf) -> Option<u64> {
    let reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    let channels = u64::from(spec.channels);
    let sample_rate = u64::from(spec.sample_rate);
    if channels == 0 || sample_rate == 0 {
        return None;
    }
    Some(u64::from(reader.duration()).saturating_mul(1_000) / channels / sample_rate)
}

fn available_audio_path(path: &PathBuf) -> Option<String> {
    if path.exists() {
        Some(path.to_string_lossy().into_owned())
    } else {
        None
    }
}

fn friendly_transcription_error() -> String {
    "Transcription failed. Check your selected transcript model and provider settings.".to_string()
}

fn friendly_recording_too_short_error() -> String {
    "Recording is too short. Hold the hotkey for more than 1 second and try again.".to_string()
}

fn friendly_inject_error() -> String {
    "The text was created but could not be pasted automatically.".to_string()
}

fn friendly_timeout_error() -> String {
    "Processing timed out before the transcript could finish.".to_string()
}

fn friendly_transform_timeout_error() -> String {
    "Transform timed out, so the original transcript was used.".to_string()
}

fn friendly_transform_error() -> String {
    "Transform failed, so the original transcript was used.".to_string()
}
