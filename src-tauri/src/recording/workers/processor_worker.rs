use crate::config::{AudioCaptureConfig, OutputConfig};
use crate::contracts::errors::RecordingErrorCode;
use crate::contracts::events::RecordingEvent;
use crate::inject;
use crate::openrouter::OpenRouterClient;
use crate::recording::command_bus::CommandBusTx;
use anyhow::Result;
use std::path::PathBuf;
use std::time::Instant as StdInstant;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::{self, timeout};

const PROCESSOR_WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

enum ProcessorWorkerCommand {
    Run {
        audio_cfg: AudioCaptureConfig,
        output_cfg: OutputConfig,
        wav_file: PathBuf,
        openrouter: Option<OpenRouterClient>,
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
        openrouter: Option<OpenRouterClient>,
    ) -> bool {
        self.command_tx
            .send(ProcessorWorkerCommand::Run {
                audio_cfg,
                output_cfg,
                wav_file,
                openrouter,
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
    let (result_tx, mut result_rx) = mpsc::unbounded_channel::<Result<(), (RecordingErrorCode, String)>>();
    let mut processing_task: Option<JoinHandle<()>> = None;

    loop {
        tokio::select! {
            Some(command) = command_rx.recv() => {
                match command {
                    ProcessorWorkerCommand::Run {
                        audio_cfg,
                        output_cfg,
                        wav_file,
                        openrouter,
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
                                process_recording_work(audio_cfg, output_cfg, wav_file, openrouter).await;
                            let success = result.is_ok();
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
                    Err((code, reason)) => RecordingEvent::ProcessFailed { code, reason },
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
    audio_cfg: crate::config::AudioCaptureConfig,
    output_cfg: crate::config::OutputConfig,
    wav_file: PathBuf,
    openrouter: Option<OpenRouterClient>,
) -> Result<(), (RecordingErrorCode, String)> {
    if wav_file.as_os_str().is_empty() {
        return Err((
            RecordingErrorCode::Processing,
            "missing recording path".to_string(),
        ));
    }

    let work = async {
        match openrouter {
            Some(client) => match client.transcribe(&wav_file).await {
                Ok(text) => match inject::deliver_text(&audio_cfg, &output_cfg, &text).await {
                    Ok(_) => Ok(()),
                    Err(err) => Err((
                        RecordingErrorCode::Processing,
                        format!("inject error: {err}"),
                    )),
                },
                Err(err) => Err((RecordingErrorCode::Processing, format!("transcription error: {err}"))),
            },
            None => Err((
                RecordingErrorCode::Processing,
                "provider not configured".to_string(),
            )),
        }
    };

    let result = if output_cfg.processing_timeout_ms == 0 {
        work.await
    } else {
        match time::timeout(
            Duration::from_millis(output_cfg.processing_timeout_ms),
            work,
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err((
                RecordingErrorCode::WorkerTimeout,
                format!("processing timeout after {}ms", output_cfg.processing_timeout_ms),
            )),
        }
    };

    if result.is_ok() && output_cfg.cleanup_recording_after_processing {
        if let Err(err) = std::fs::remove_file(&wav_file) {
            log::warn!("failed to cleanup audio artifact {:?}: {:?}", wav_file, err);
        } else {
            log::debug!("removed audio artifact {:?}", wav_file);
        }
    } else if wav_file.exists() {
        log::trace!("audio artifact retained by policy: {:?}", wav_file);
    }

    result
}
