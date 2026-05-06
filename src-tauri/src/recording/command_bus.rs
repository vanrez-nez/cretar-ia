use crate::config::AppConfig;
use crate::contracts::commands::RecordingCommand;
use crate::contracts::events::{HotkeyEvent, RecordingEvent};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc::{self, error::TrySendError, Receiver, Sender};

pub const HOTKEY_QUEUE_CAPACITY: usize = 64;
pub const WORKER_QUEUE_CAPACITY: usize = 128;
const COMMAND_QUEUE_CAPACITY: usize = 64;

#[derive(Clone)]
pub struct CommandBusTx {
    hotkey_tx: Sender<HotkeyEvent>,
    worker_tx: Sender<RecordingEvent>,
    command_tx: Sender<RecordingCommand>,
    hotkey_dropped: Arc<AtomicU32>,
    worker_dropped: Arc<AtomicU32>,
    command_dropped: Arc<AtomicU32>,
    hotkey_paused: Arc<AtomicBool>,
}

pub struct CommandBus {
    pub hotkey_rx: Receiver<HotkeyEvent>,
    pub worker_rx: Receiver<RecordingEvent>,
    pub command_rx: Receiver<RecordingCommand>,
    pub tx: CommandBusTx,
}

impl CommandBus {
    pub fn new(cfg: &AppConfig) -> Self {
        let hotkey_capacity = normalized_capacity(
            cfg.effective_hotkey_queue_capacity(),
            HOTKEY_QUEUE_CAPACITY,
        );
        let worker_capacity = normalized_capacity(
            cfg.effective_worker_queue_capacity(),
            WORKER_QUEUE_CAPACITY,
        );

        let (hotkey_tx, hotkey_rx) = mpsc::channel::<HotkeyEvent>(hotkey_capacity);
        let (worker_tx, worker_rx) = mpsc::channel::<RecordingEvent>(worker_capacity);
        let (command_tx, command_rx) = mpsc::channel::<RecordingCommand>(COMMAND_QUEUE_CAPACITY);

        Self {
            hotkey_rx,
            worker_rx,
            command_rx,
            tx: CommandBusTx {
                hotkey_tx,
                worker_tx,
                command_tx,
                hotkey_dropped: Arc::new(AtomicU32::new(0)),
                worker_dropped: Arc::new(AtomicU32::new(0)),
                command_dropped: Arc::new(AtomicU32::new(0)),
                hotkey_paused: Arc::new(AtomicBool::new(false)),
            },
        }
    }

    pub fn sender(&self) -> CommandBusTx {
        self.tx.clone()
    }

    pub fn close_and_drain(&mut self) -> DrainedCommandBus {
        self.hotkey_rx.close();
        self.worker_rx.close();
        self.command_rx.close();

        DrainedCommandBus {
            hotkey: drain_receiver(&mut self.hotkey_rx),
            worker: drain_receiver(&mut self.worker_rx),
            command: drain_receiver(&mut self.command_rx),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainedCommandBus {
    pub hotkey: usize,
    pub worker: usize,
    pub command: usize,
}

fn drain_receiver<T>(rx: &mut Receiver<T>) -> usize {
    let mut drained = 0usize;
    while rx.try_recv().is_ok() {
        drained = drained.saturating_add(1);
    }
    drained
}

fn normalized_capacity(configured: u32, fallback: usize) -> usize {
    if configured > 0 {
        configured.max(1) as usize
    } else {
        fallback
    }
}

impl CommandBusTx {
    pub fn send_hotkey(&self, event: HotkeyEvent) -> Option<RecordingEvent> {
        if self.hotkey_paused.load(Ordering::Relaxed) {
            log::trace!("hotkey event ignored while runtime hotkeys are paused for settings");
            return None;
        }

        match self.hotkey_tx.try_send(event) {
            Ok(()) => None,
            Err(TrySendError::Full(_)) => Some(self.queue_saturated("hotkey", &self.hotkey_dropped)),
            Err(TrySendError::Closed(_)) => None,
        }
    }

    pub fn send_worker(&self, event: RecordingEvent) -> Option<RecordingEvent> {
        match self.worker_tx.try_send(event) {
            Ok(()) => None,
            Err(TrySendError::Full(_)) => Some(self.queue_saturated("worker", &self.worker_dropped)),
            Err(TrySendError::Closed(_)) => None,
        }
    }

    pub fn send_command(&self, command: RecordingCommand) -> Option<RecordingEvent> {
        match self.command_tx.try_send(command) {
            Ok(()) => None,
            Err(TrySendError::Full(_)) => Some(self.queue_saturated("command", &self.command_dropped)),
            Err(TrySendError::Closed(_)) => None,
        }
    }

    pub fn set_hotkey_paused(&self, paused: bool) {
        self.hotkey_paused.store(paused, Ordering::Relaxed);
    }

    fn queue_saturated(&self, source: &str, metric: &AtomicU32) -> RecordingEvent {
        let dropped = metric.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        RecordingEvent::QueueSaturated {
            source: source.to_string(),
            dropped,
        }
    }
}
