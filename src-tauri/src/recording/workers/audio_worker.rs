use crate::audio::Recorder;
use crate::config::AudioCaptureConfig;
use crate::contracts::errors::RecordingErrorCode;
use crate::contracts::events::RecordingEvent;
use crate::recording::command_bus::CommandBusTx;
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

const AUDIO_WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
enum AudioWorkerCommand {
    Start {
        cfg: AudioCaptureConfig,
        record_base: PathBuf,
    },
    Stop,
    ForceStop,
    Shutdown {
        ack: oneshot::Sender<()>,
    },
}

#[derive(Clone)]
pub struct AudioWorkerHandle {
    command_tx: UnboundedSender<AudioWorkerCommand>,
}

impl AudioWorkerHandle {
    pub fn request_force_stop(&self) -> bool {
        self.command_tx.send(AudioWorkerCommand::ForceStop).is_ok()
    }
}

#[derive(Debug)]
pub struct AudioWorker {
    command_tx: mpsc::UnboundedSender<AudioWorkerCommand>,
    handle: JoinHandle<()>,
}

impl AudioWorker {
    pub fn start(bus_tx: CommandBusTx) -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let tx = bus_tx.clone();

        let handle = tokio::task::spawn_blocking(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    log::error!("failed to start audio worker runtime: {err}");
                    return;
                }
            };

            runtime.block_on(worker_loop(command_rx, tx));
        });

        Self {
            command_tx,
            handle,
        }
    }

    pub fn handle(&self) -> AudioWorkerHandle {
        AudioWorkerHandle {
            command_tx: self.command_tx.clone(),
        }
    }

    pub fn request_start(&self, cfg: AudioCaptureConfig, record_base: PathBuf) -> bool {
        self.command_tx
            .send(AudioWorkerCommand::Start { cfg, record_base })
            .is_ok()
    }

    pub fn request_stop(&self) -> bool {
        self.command_tx.send(AudioWorkerCommand::Stop).is_ok()
    }

    pub fn request_force_stop(&self) -> bool {
        self.command_tx.send(AudioWorkerCommand::ForceStop).is_ok()
    }

    pub async fn shutdown(self) {
        let (ack_tx, ack_rx) = oneshot::channel();
        if self
            .command_tx
            .send(AudioWorkerCommand::Shutdown { ack: ack_tx })
            .is_ok()
        {
            let _ = timeout(AUDIO_WORKER_SHUTDOWN_TIMEOUT, ack_rx).await;
        }
        let _ = timeout(AUDIO_WORKER_SHUTDOWN_TIMEOUT, self.handle).await;
    }
}

async fn worker_loop(mut command_rx: UnboundedReceiver<AudioWorkerCommand>, tx: CommandBusTx) {
    let mut active_recorder: Option<Recorder> = None;

    while let Some(command) = command_rx.recv().await {
        match command {
            AudioWorkerCommand::Start { cfg, record_base } => {
                if active_recorder.is_some() {
                if tx
                    .send_worker(RecordingEvent::AudioStartFailed {
                            code: RecordingErrorCode::AudioInit,
                            reason: "start requested while recorder already active".to_string(),
                        })
                        .is_some()
                    {
                        log::warn!(
                            "audio start failure dropped because worker queue was full"
                        );
                    }
                    continue;
                }

                match Recorder::start(&cfg, record_base, tx.clone()) {
                    Ok(recorder) => {
                        active_recorder = Some(recorder);
                        if tx.send_worker(RecordingEvent::AudioStarted).is_some() {
                            log::warn!("audio started event dropped because worker queue was full");
                        }
                    }
                    Err(err) => {
                        let reason = format!("audio start failed: {err}");
                        let event = if is_audio_device_unavailable_error(&reason) {
                            RecordingEvent::AudioDeviceUnavailable {
                                code: RecordingErrorCode::AudioInit,
                                reason,
                            }
                        } else {
                            RecordingEvent::AudioStartFailed {
                                code: RecordingErrorCode::AudioInit,
                                reason,
                            }
                        };
                        if tx
                            .send_worker(event)
                            .is_some()
                        {
                            log::warn!("audio start failure event dropped because worker queue was full");
                        }
                    }
                }
            }

            AudioWorkerCommand::Stop => {
                active_recorder = stop_active_recorder(active_recorder.take(), tx.clone());
            }

            AudioWorkerCommand::ForceStop => {
                active_recorder = stop_active_recorder(active_recorder.take(), tx.clone());
            }

            AudioWorkerCommand::Shutdown { ack } => {
                let _ = stop_active_recorder(active_recorder.take(), tx.clone());
                let _ = ack.send(());
                break;
            }
        }
    }
}

fn is_audio_device_unavailable_error(reason: &str) -> bool {
    reason.contains("configured audio input device")
        || reason.contains("no default input device found")
        || reason.contains("device is no longer available")
}

fn stop_active_recorder(recorder: Option<Recorder>, tx: CommandBusTx) -> Option<Recorder> {
    let Some(recorder) = recorder else {
        return None;
    };

    match recorder.stop() {
        Ok(artifact) => {
            if tx
                .send_worker(RecordingEvent::AudioStopped { artifact })
                .is_some()
            {
                log::warn!("audio stopped event dropped because worker queue was full");
            }
            None
        }
        Err(err) => {
            if tx
                .send_worker(RecordingEvent::AudioStopFailed {
                    code: RecordingErrorCode::AudioStop,
                    reason: format!("stop recorder failed: {err}"),
                })
                .is_some()
            {
                log::warn!("audio stop failure event dropped because worker queue was full");
            }
            None
        }
    }
}
