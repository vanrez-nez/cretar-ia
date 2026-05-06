use crate::audio::Recorder;
use crate::config::AudioCaptureConfig;
use crate::contracts::errors::RecordingErrorCode;
use crate::contracts::events::RecordingEvent;
use crate::recording::command_bus::CommandBusTx;
use std::path::PathBuf;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

#[derive(Debug)]
enum AudioWorkerCommand {
    Start {
        cfg: AudioCaptureConfig,
        record_base: PathBuf,
    },
    Stop,
    ForceStop,
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

        let handle = tokio::spawn(async move {
            worker_loop(command_rx, tx).await;
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
        self.request_force_stop();
        let _ = self.handle.await;
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

                match tokio::task::spawn_blocking(move || Recorder::start(&cfg, record_base)).await {
                    Ok(Ok(recorder)) => {
                        active_recorder = Some(recorder);
                        if tx.send_worker(RecordingEvent::AudioStarted).is_some() {
                            log::warn!("audio started event dropped because worker queue was full");
                        }
                    }
                    Ok(Err(err)) => {
                        if tx
                            .send_worker(RecordingEvent::AudioStartFailed {
                                code: RecordingErrorCode::AudioInit,
                                reason: format!("audio start failed: {err}"),
                            })
                            .is_some()
                        {
                            log::warn!("audio start failure event dropped because worker queue was full");
                        }
                    }
                    Err(err) => {
                        if tx
                            .send_worker(RecordingEvent::AudioStartFailed {
                                code: RecordingErrorCode::Unknown,
                                reason: format!("audio start worker failed: {err}"),
                            })
                            .is_some()
                        {
                            log::warn!("audio start failure event dropped because worker queue was full");
                        }
                    }
                }
            }

            AudioWorkerCommand::Stop => {
                active_recorder = stop_active_recorder(active_recorder.take(), tx.clone()).await;
            }

            AudioWorkerCommand::ForceStop => {
                active_recorder = stop_active_recorder(active_recorder.take(), tx.clone()).await;
            }
        }
    }
}

async fn stop_active_recorder(recorder: Option<Recorder>, tx: CommandBusTx) -> Option<Recorder> {
    let Some(recorder) = recorder else {
        return None;
    };

    match tokio::task::spawn_blocking(move || recorder.stop()).await {
        Ok(Ok(path)) => {
            if tx
                .send_worker(RecordingEvent::AudioStopped { path })
                .is_some()
            {
                log::warn!("audio stopped event dropped because worker queue was full");
            }
            None
        }
        Ok(Err(err)) => {
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
        Err(err) => {
            if tx
                .send_worker(RecordingEvent::AudioStopFailed {
                    code: RecordingErrorCode::Unknown,
                    reason: format!("stop worker failed: {err}"),
                })
                .is_some()
            {
                log::warn!("audio stop failure event dropped because worker queue was full");
            }
            None
        }
    }
}
