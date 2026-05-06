use crate::contracts::errors::RecordingErrorCode;
use crate::contracts::events::RecordingEvent;
use crate::recording::command_bus::CommandBusTx;
use crate::recording::workers::{audio_worker::AudioWorkerHandle, processor_worker::ProcessorWorkerHandle};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

enum RecoveryCommand {
    Recover,
}

#[derive(Clone)]
pub struct RecoveryWorker {
    command_tx: UnboundedSender<RecoveryCommand>,
    handle: JoinHandle<()>,
}

impl RecoveryWorker {
    pub fn start(
        bus_tx: CommandBusTx,
        audio_worker: AudioWorkerHandle,
        processor_worker: ProcessorWorkerHandle,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let tx = bus_tx.clone();

        let handle = tokio::spawn(async move {
            worker_loop(command_rx, tx, audio_worker, processor_worker).await;
        });

        Self {
            command_tx,
            handle,
        }
    }

    pub fn request_recovery(&self) -> bool {
        self.command_tx.send(RecoveryCommand::Recover).is_ok()
    }

    pub async fn shutdown(self) {
        drop(self.command_tx);
        let _ = self.handle.await;
    }
}

async fn worker_loop(
    mut command_rx: UnboundedReceiver<RecoveryCommand>,
    tx: CommandBusTx,
    audio_worker: AudioWorkerHandle,
    processor_worker: ProcessorWorkerHandle,
) {
    let mut in_progress = false;

    while let Some(command) = command_rx.recv().await {
        if let RecoveryCommand::Recover = command {
            if in_progress {
                continue;
            }
            in_progress = true;

            let audio_stopped = audio_worker.request_force_stop();
            let processor_aborted = processor_worker.request_cancel();
            if !audio_stopped || !processor_aborted {
                if tx
                    .send_worker(RecordingEvent::RecoveryFailed {
                        code: RecordingErrorCode::Unknown,
                        reason: "recovery dispatch failed".to_string(),
                    })
                    .is_some()
                {
                    log::warn!("recovery failure event dropped because worker queue was full");
                }
                in_progress = false;
                continue;
            }

            if tx.send_worker(RecordingEvent::RecoveryCompleted).is_some() {
                log::warn!("recovery completion event dropped because worker queue was full");
            }
            in_progress = false;
        }
    }
}
