use crate::config::{AudioCaptureConfig, OutputConfig};
use crate::contracts::errors::RecordingErrorCode;
use crate::contracts::events::{RecordingArtifact, RecordingEvent};
use crate::history::{HistoryEntry, HistoryStore};
use crate::inject;
use crate::model_health::ModelHealthStatus;
use crate::prompts::PromptView;
use crate::providers::{DynFormattingProvider, DynSpeechToTextProvider};
use crate::recording::command_bus::CommandBusTx;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::Instant as StdInstant;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use voice_activity_detector::{IteratorExt, VoiceActivityDetector};

const PROCESSOR_WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const MIN_TRANSCRIPTION_AUDIO_DURATION_MS: u64 = 1_000;
const VAD_SAMPLE_RATE: u32 = 16_000;
const VAD_CHUNK_SIZE: usize = 512;
const VAD_THRESHOLD: f32 = 0.5;
const VAD_PADDING_CHUNKS: usize = 3;
const VAD_MIN_SPEECH_CHUNKS: usize = 1;

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
        artifact: RecordingArtifact,
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
        artifact: RecordingArtifact,
        stt_provider: Option<DynSpeechToTextProvider>,
        transform: TransformRuntime,
        history: Option<HistoryStore>,
    ) -> bool {
        self.command_tx
            .send(ProcessorWorkerCommand::Run {
                audio_cfg,
                output_cfg,
                artifact,
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
    let (result_tx, mut result_rx) = mpsc::unbounded_channel::<ProcessorWorkOutcome>();
    let mut processing_task: Option<JoinHandle<()>> = None;

    loop {
        tokio::select! {
            Some(command) = command_rx.recv() => {
                match command {
                    ProcessorWorkerCommand::Run {
                        audio_cfg,
                        output_cfg,
                        artifact,
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
                                    artifact,
                                    stt_provider,
                                    transform,
                                    history,
                                )
                                .await;
                            let success = matches!(result, ProcessorWorkOutcome::Completed);
                            if let ProcessorWorkOutcome::Failed { code, reason } = &result {
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
                    ProcessorWorkOutcome::Completed => RecordingEvent::ProcessCompleted,
                    ProcessorWorkOutcome::Cancelled { reason } => {
                        RecordingEvent::RecordingCancelled { reason }
                    }
                    ProcessorWorkOutcome::Failed { code, reason } => {
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
    artifact: RecordingArtifact,
    stt_provider: Option<DynSpeechToTextProvider>,
    transform: TransformRuntime,
    history: Option<HistoryStore>,
) -> ProcessorWorkOutcome {
    process_recording_work_with_vad(
        audio_cfg,
        output_cfg,
        artifact,
        stt_provider,
        transform,
        history,
        run_vad_gate,
    )
    .await
}

async fn process_recording_work_with_vad<F>(
    audio_cfg: AudioCaptureConfig,
    output_cfg: OutputConfig,
    artifact: RecordingArtifact,
    stt_provider: Option<DynSpeechToTextProvider>,
    transform: TransformRuntime,
    history: Option<HistoryStore>,
    vad_gate: F,
) -> ProcessorWorkOutcome
where
    F: FnOnce(&Path) -> VadGateReport,
{
    let wav_file = artifact.path.clone();
    if wav_file.as_os_str().is_empty() {
        return ProcessorWorkOutcome::Failed {
            code: RecordingErrorCode::Processing,
            reason: "missing recording path".to_string(),
        };
    }
    let audio_duration_ms = if artifact.duration_ms > 0 {
        artifact.duration_ms
    } else {
        wav_duration_ms(&wav_file).unwrap_or(0)
    };

    if audio_duration_ms <= MIN_TRANSCRIPTION_AUDIO_DURATION_MS {
        log::warn!(
            "recording too short for transcription: duration_ms={} min_duration_ms={}",
            audio_duration_ms,
            MIN_TRANSCRIPTION_AUDIO_DURATION_MS
        );
        cleanup_audio_artifact(&wav_file);
        return ProcessorWorkOutcome::Cancelled {
            reason: "recording_too_short".to_string(),
        };
    }

    let vad_report = vad_gate(&wav_file);
    log_vad_gate(audio_duration_ms, &vad_report);
    match vad_report.result {
        VadGateResult::Speech | VadGateResult::FailOpen => {}
        VadGateResult::NoSpeech => {
            cleanup_audio_artifact(&wav_file);
            return ProcessorWorkOutcome::Cancelled {
                reason: "recording_silent".to_string(),
            };
        }
    }

    let mut record = ProcessRecord::empty();
    let result: Result<(), (RecordingErrorCode, String)> = match run_transcript_step(
        stt_provider,
        &wav_file,
        output_cfg.processing_timeout_ms,
    )
    .await
    {
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
    };

    if output_cfg.cleanup_recording_after_processing {
        cleanup_audio_artifact(&wav_file);
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

    match result {
        Ok(()) => ProcessorWorkOutcome::Completed,
        Err((code, reason)) => ProcessorWorkOutcome::Failed { code, reason },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProcessorWorkOutcome {
    Completed,
    Cancelled {
        reason: String,
    },
    Failed {
        code: RecordingErrorCode,
        reason: String,
    },
}

fn cleanup_audio_artifact(wav_file: &PathBuf) {
    if !wav_file.exists() {
        log::trace!("audio artifact already removed: {:?}", wav_file);
    } else if let Err(err) = std::fs::remove_file(wav_file) {
        log::warn!("failed to cleanup audio artifact {:?}: {:?}", wav_file, err);
    } else {
        log::debug!("removed audio artifact {:?}", wav_file);
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VadGateResult {
    Speech,
    NoSpeech,
    FailOpen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VadGateReport {
    result: VadGateResult,
    vad_ms: u128,
    source_sample_rate: u32,
    source_channels: u16,
    vad_sample_count: usize,
    speech_chunks: usize,
    total_chunks: usize,
    error: Option<String>,
}

struct PreparedVadAudio {
    samples: Vec<i16>,
    source_sample_rate: u32,
    source_channels: u16,
}

fn run_vad_gate(wav_file: &Path) -> VadGateReport {
    let started_at = StdInstant::now();
    let prepared = match prepare_vad_audio(wav_file) {
        Ok(prepared) => prepared,
        Err(err) => {
            return VadGateReport {
                result: VadGateResult::FailOpen,
                vad_ms: started_at.elapsed().as_millis(),
                source_sample_rate: 0,
                source_channels: 0,
                vad_sample_count: 0,
                speech_chunks: 0,
                total_chunks: 0,
                error: Some(err.to_string()),
            };
        }
    };

    let source_sample_rate = prepared.source_sample_rate;
    let source_channels = prepared.source_channels;
    let vad_sample_count = prepared.samples.len();
    let mut vad = match VoiceActivityDetector::builder()
        .sample_rate(VAD_SAMPLE_RATE)
        .chunk_size(VAD_CHUNK_SIZE)
        .build()
    {
        Ok(vad) => vad,
        Err(err) => {
            return VadGateReport {
                result: VadGateResult::FailOpen,
                vad_ms: started_at.elapsed().as_millis(),
                source_sample_rate,
                source_channels,
                vad_sample_count,
                speech_chunks: 0,
                total_chunks: 0,
                error: Some(err.to_string()),
            };
        }
    };

    let mut speech_chunks = 0usize;
    let mut total_chunks = 0usize;
    for chunk in prepared
        .samples
        .into_iter()
        .label(&mut vad, VAD_THRESHOLD, VAD_PADDING_CHUNKS)
    {
        total_chunks += 1;
        if chunk.is_speech() {
            speech_chunks += 1;
            if speech_chunks >= VAD_MIN_SPEECH_CHUNKS {
                break;
            }
        }
    }

    VadGateReport {
        result: if speech_chunks >= VAD_MIN_SPEECH_CHUNKS {
            VadGateResult::Speech
        } else {
            VadGateResult::NoSpeech
        },
        vad_ms: started_at.elapsed().as_millis(),
        source_sample_rate,
        source_channels,
        vad_sample_count,
        speech_chunks,
        total_chunks,
        error: None,
    }
}

fn log_vad_gate(duration_ms: u64, report: &VadGateReport) {
    let result = match report.result {
        VadGateResult::Speech => "speech",
        VadGateResult::NoSpeech => "no_speech",
        VadGateResult::FailOpen => "fail_open",
    };
    if let Some(error) = report.error.as_deref() {
        log::warn!("vad gate failed open: {error}");
    }
    log::info!(
        "profile.vad_gate result={} duration_ms={} vad_ms={} source_sample_rate={} source_channels={} vad_sample_count={} speech_chunks={} total_chunks={} threshold={} padding_chunks={} error={:?}",
        result,
        duration_ms,
        report.vad_ms,
        report.source_sample_rate,
        report.source_channels,
        report.vad_sample_count,
        report.speech_chunks,
        report.total_chunks,
        VAD_THRESHOLD,
        VAD_PADDING_CHUNKS,
        report.error
    );
}

fn prepare_vad_audio(wav_file: &Path) -> Result<PreparedVadAudio> {
    let mut reader = hound::WavReader::open(wav_file)
        .with_context(|| format!("opening wav for VAD: {}", wav_file.display()))?;
    let spec = reader.spec();
    if spec.channels == 0 {
        return Err(anyhow::anyhow!("wav has zero channels"));
    }
    if spec.sample_rate == 0 {
        return Err(anyhow::anyhow!("wav has zero sample rate"));
    }

    let mono = read_mono_samples(&mut reader, spec)?;
    let vad_samples = if spec.sample_rate == VAD_SAMPLE_RATE {
        mono
    } else {
        resample_linear(&mono, spec.sample_rate, VAD_SAMPLE_RATE)
    };
    let samples = vad_samples.into_iter().map(float_to_i16).collect();

    Ok(PreparedVadAudio {
        samples,
        source_sample_rate: spec.sample_rate,
        source_channels: spec.channels,
    })
}

fn read_mono_samples<R: std::io::Read>(
    reader: &mut hound::WavReader<R>,
    spec: hound::WavSpec,
) -> Result<Vec<f32>> {
    match spec.sample_format {
        hound::SampleFormat::Float => {
            read_mono_from_samples(reader.samples::<f32>(), spec.channels, |sample| {
                sample.clamp(-1.0, 1.0)
            })
        }
        hound::SampleFormat::Int if spec.bits_per_sample <= 8 => {
            read_mono_from_samples(reader.samples::<i8>(), spec.channels, |sample| {
                sample as f32 / i8::MAX as f32
            })
        }
        hound::SampleFormat::Int if spec.bits_per_sample <= 16 => {
            read_mono_from_samples(reader.samples::<i16>(), spec.channels, |sample| {
                sample as f32 / i16::MAX as f32
            })
        }
        hound::SampleFormat::Int if spec.bits_per_sample <= 32 => {
            let max = ((1_i64 << (spec.bits_per_sample - 1)) - 1) as f32;
            read_mono_from_samples(reader.samples::<i32>(), spec.channels, |sample| {
                (sample as f32 / max).clamp(-1.0, 1.0)
            })
        }
        _ => Err(anyhow::anyhow!(
            "unsupported wav format for VAD: {:?} {} bits",
            spec.sample_format,
            spec.bits_per_sample
        )),
    }
}

fn read_mono_from_samples<S, I, F>(
    samples: I,
    channels: u16,
    mut sample_to_f32: F,
) -> Result<Vec<f32>>
where
    I: Iterator<Item = std::result::Result<S, hound::Error>>,
    F: FnMut(S) -> f32,
{
    let channels = usize::from(channels);
    let mut mono = Vec::new();
    let mut frame_sum = 0.0f32;
    let mut channel_index = 0usize;

    for sample in samples {
        frame_sum += sample_to_f32(sample.context("reading wav sample for VAD")?);
        channel_index += 1;
        if channel_index == channels {
            mono.push(frame_sum / channels as f32);
            frame_sum = 0.0;
            channel_index = 0;
        }
    }

    if channel_index > 0 {
        mono.push(frame_sum / channel_index as f32);
    }

    Ok(mono)
}

fn resample_linear(samples: &[f32], source_sample_rate: u32, target_sample_rate: u32) -> Vec<f32> {
    if samples.is_empty() || source_sample_rate == 0 || target_sample_rate == 0 {
        return Vec::new();
    }
    if source_sample_rate == target_sample_rate {
        return samples.to_vec();
    }

    let target_len = ((samples.len() as u128 * u128::from(target_sample_rate))
        / u128::from(source_sample_rate))
    .max(1) as usize;
    let ratio = source_sample_rate as f64 / target_sample_rate as f64;
    let mut output = Vec::with_capacity(target_len);

    for target_index in 0..target_len {
        let source_pos = target_index as f64 * ratio;
        let left_index = source_pos.floor() as usize;
        let right_index = (left_index + 1).min(samples.len() - 1);
        let frac = (source_pos - left_index as f64) as f32;
        let sample = samples[left_index] * (1.0 - frac) + samples[right_index] * frac;
        output.push(sample);
    }

    output
}

fn float_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn friendly_transcription_error() -> String {
    "Transcription failed. Check your selected transcript model and provider settings.".to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OutputConfig;
    use crate::providers::SpeechToTextProvider;
    use hound::{SampleFormat, WavSpec, WavWriter};
    use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
    use std::fs;
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use uuid::Uuid;

    #[tokio::test]
    async fn short_recordings_do_not_write_history_or_retain_audio() -> anyhow::Result<()> {
        let pool = history_pool().await?;
        let history = HistoryStore::new(pool.clone());
        let test_dir =
            std::env::temp_dir().join(format!("cretar-ia-short-recording-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&test_dir)?;
        let wav_file = test_dir.join("short.wav");
        write_test_wav(&wav_file, 500)?;

        let result = process_recording_work(
            AudioCaptureConfig::default(),
            OutputConfig {
                cleanup_recording_after_processing: false,
                ..OutputConfig::default()
            },
            test_artifact(&wav_file, 500),
            None,
            TransformRuntime::disabled(),
            Some(history),
        )
        .await;

        assert_eq!(
            result,
            ProcessorWorkOutcome::Cancelled {
                reason: "recording_too_short".to_string(),
            }
        );

        assert_eq!(history_row_count(&pool).await?, 0);
        assert!(
            !wav_file.exists(),
            "short recording artifact should be removed even when retention is enabled"
        );

        fs::remove_dir_all(&test_dir)?;
        Ok(())
    }

    #[tokio::test]
    async fn no_speech_vad_result_does_not_call_stt_write_history_or_retain_audio(
    ) -> anyhow::Result<()> {
        let pool = history_pool().await?;
        let history = HistoryStore::new(pool.clone());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(CountingSttProvider {
            calls: Arc::clone(&calls),
        });
        let test_dir = std::env::temp_dir().join(format!(
            "cretar-ia-silent-recording-test-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&test_dir)?;
        let wav_file = test_dir.join("silent.wav");
        write_test_wav(&wav_file, 1_500)?;

        let result = process_recording_work_with_vad(
            AudioCaptureConfig::default(),
            OutputConfig {
                cleanup_recording_after_processing: false,
                ..OutputConfig::default()
            },
            test_artifact(&wav_file, 1_500),
            Some(provider),
            TransformRuntime::disabled(),
            Some(history),
            |_| vad_report(VadGateResult::NoSpeech),
        )
        .await;

        assert_eq!(
            result,
            ProcessorWorkOutcome::Cancelled {
                reason: "recording_silent".to_string(),
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(history_row_count(&pool).await?, 0);
        assert!(
            !wav_file.exists(),
            "silent recording artifact should be removed even when retention is enabled"
        );

        fs::remove_dir_all(&test_dir)?;
        Ok(())
    }

    #[tokio::test]
    async fn speech_vad_result_continues_to_transcription_step() -> anyhow::Result<()> {
        let test_dir =
            std::env::temp_dir().join(format!("cretar-ia-speech-vad-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&test_dir)?;
        let wav_file = test_dir.join("speech.wav");
        write_test_wav(&wav_file, 1_500)?;

        let result = process_recording_work_with_vad(
            AudioCaptureConfig::default(),
            OutputConfig {
                cleanup_recording_after_processing: false,
                ..OutputConfig::default()
            },
            test_artifact(&wav_file, 1_500),
            None,
            TransformRuntime::disabled(),
            None,
            |_| vad_report(VadGateResult::Speech),
        )
        .await;

        assert_eq!(
            result,
            ProcessorWorkOutcome::Failed {
                code: RecordingErrorCode::Processing,
                reason: "Choose a transcript model before recording.".to_string(),
            }
        );
        assert!(wav_file.exists());

        fs::remove_dir_all(&test_dir)?;
        Ok(())
    }

    #[tokio::test]
    async fn vad_failure_fails_open_to_transcription_step() -> anyhow::Result<()> {
        let test_dir =
            std::env::temp_dir().join(format!("cretar-ia-vad-fail-open-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&test_dir)?;
        let wav_file = test_dir.join("speech.wav");
        write_test_wav(&wav_file, 1_500)?;

        let result = process_recording_work_with_vad(
            AudioCaptureConfig::default(),
            OutputConfig {
                cleanup_recording_after_processing: false,
                ..OutputConfig::default()
            },
            test_artifact(&wav_file, 1_500),
            None,
            TransformRuntime::disabled(),
            None,
            |_| vad_report(VadGateResult::FailOpen),
        )
        .await;

        assert_eq!(
            result,
            ProcessorWorkOutcome::Failed {
                code: RecordingErrorCode::Processing,
                reason: "Choose a transcript model before recording.".to_string(),
            }
        );
        assert!(wav_file.exists());

        fs::remove_dir_all(&test_dir)?;
        Ok(())
    }

    #[test]
    fn vad_preparation_keeps_16khz_mono_sample_count() -> anyhow::Result<()> {
        let test_dir =
            std::env::temp_dir().join(format!("cretar-ia-vad-prepare-mono-{}", Uuid::new_v4()));
        fs::create_dir_all(&test_dir)?;
        let wav_file = test_dir.join("mono.wav");
        write_test_wav_with_format(&wav_file, 100, 16_000, 1, 1_000)?;

        let prepared = prepare_vad_audio(&wav_file)?;

        assert_eq!(prepared.source_sample_rate, 16_000);
        assert_eq!(prepared.source_channels, 1);
        assert_eq!(prepared.samples.len(), 1_600);

        fs::remove_dir_all(&test_dir)?;
        Ok(())
    }

    #[test]
    fn vad_preparation_downmixes_stereo_to_mono() -> anyhow::Result<()> {
        let test_dir =
            std::env::temp_dir().join(format!("cretar-ia-vad-prepare-stereo-{}", Uuid::new_v4()));
        fs::create_dir_all(&test_dir)?;
        let wav_file = test_dir.join("stereo.wav");
        write_stereo_test_wav(&wav_file)?;

        let prepared = prepare_vad_audio(&wav_file)?;

        assert_eq!(prepared.source_channels, 2);
        assert_eq!(prepared.samples.len(), 16);
        assert!(
            (15_000..=17_000).contains(&prepared.samples[0]),
            "expected downmixed sample around half scale, got {}",
            prepared.samples[0]
        );

        fs::remove_dir_all(&test_dir)?;
        Ok(())
    }

    #[test]
    fn vad_preparation_resamples_to_16khz() -> anyhow::Result<()> {
        let test_dir =
            std::env::temp_dir().join(format!("cretar-ia-vad-prepare-resample-{}", Uuid::new_v4()));
        fs::create_dir_all(&test_dir)?;
        let wav_file = test_dir.join("resample.wav");
        write_test_wav_with_format(&wav_file, 100, 24_000, 1, 1_000)?;

        let prepared = prepare_vad_audio(&wav_file)?;

        assert_eq!(prepared.source_sample_rate, 24_000);
        assert_eq!(prepared.samples.len(), 1_600);

        fs::remove_dir_all(&test_dir)?;
        Ok(())
    }

    #[test]
    fn vad_preparation_rejects_invalid_wav() -> anyhow::Result<()> {
        let test_dir =
            std::env::temp_dir().join(format!("cretar-ia-vad-prepare-invalid-{}", Uuid::new_v4()));
        fs::create_dir_all(&test_dir)?;
        let wav_file = test_dir.join("invalid.wav");
        fs::write(&wav_file, b"not a wav")?;

        assert!(prepare_vad_audio(&wav_file).is_err());

        fs::remove_dir_all(&test_dir)?;
        Ok(())
    }

    async fn history_pool() -> anyhow::Result<SqlitePool> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::query(
            "CREATE TABLE history (
                id TEXT PRIMARY KEY,
                audio_file_path TEXT,
                audio_duration_ms INTEGER NOT NULL DEFAULT 0,
                transcript_text TEXT,
                transform_text TEXT,
                error_message TEXT,
                word_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await?;
        Ok(pool)
    }

    async fn history_row_count(pool: &SqlitePool) -> anyhow::Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS count FROM history")
            .fetch_one(pool)
            .await?;
        Ok(row.try_get("count")?)
    }

    fn write_test_wav(path: &PathBuf, duration_ms: u64) -> anyhow::Result<()> {
        write_test_wav_with_format(path, duration_ms, 16_000, 1, 0)
    }

    fn write_test_wav_with_format(
        path: &PathBuf,
        duration_ms: u64,
        sample_rate: u32,
        channels: u16,
        sample: i16,
    ) -> anyhow::Result<()> {
        let sample_count = sample_rate as u64 * duration_ms / 1_000;
        let spec = WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec)?;
        for _ in 0..sample_count {
            for _ in 0..channels {
                writer.write_sample::<i16>(sample)?;
            }
        }
        writer.finalize()?;
        Ok(())
    }

    fn write_stereo_test_wav(path: &PathBuf) -> anyhow::Result<()> {
        let spec = WavSpec {
            channels: 2,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec)?;
        for _ in 0..16 {
            writer.write_sample::<i16>(i16::MAX)?;
            writer.write_sample::<i16>(0)?;
        }
        writer.finalize()?;
        Ok(())
    }

    fn test_artifact(path: &PathBuf, duration_ms: u64) -> RecordingArtifact {
        RecordingArtifact {
            path: path.clone(),
            duration_ms,
            levels: Default::default(),
        }
    }

    fn vad_report(result: VadGateResult) -> VadGateReport {
        VadGateReport {
            result,
            vad_ms: 1,
            source_sample_rate: VAD_SAMPLE_RATE,
            source_channels: 1,
            vad_sample_count: 16_000,
            speech_chunks: if result == VadGateResult::Speech {
                1
            } else {
                0
            },
            total_chunks: 1,
            error: if result == VadGateResult::FailOpen {
                Some("test vad failure".to_string())
            } else {
                None
            },
        }
    }

    struct CountingSttProvider {
        calls: Arc<AtomicUsize>,
    }

    impl SpeechToTextProvider for CountingSttProvider {
        fn transcribe<'a>(
            &'a self,
            _wav_file: &'a Path,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + 'a>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok("transcript".to_string()) })
        }
    }
}
