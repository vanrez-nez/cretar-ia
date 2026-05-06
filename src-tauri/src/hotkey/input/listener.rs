use crate::config::InteractionConfig;
use crate::recording::command_bus::CommandBusTx;
use crate::hotkey::input::traits::HotkeyInputCoordinator;
use crate::hotkey::input::stabilizer::HotkeyInputAdapter;
use anyhow::Result;
use rdev::{listen, Event, EventType};
use std::thread::JoinHandle;

use super::traits::HotkeyInputListener;

pub struct RdevHotkeyListener;

impl HotkeyInputListener for RdevHotkeyListener {
    fn start(cfg: InteractionConfig, tx: CommandBusTx) -> Result<JoinHandle<()>> {
        start_listener(cfg, tx)
    }
}

pub fn start_listener(
    cfg: InteractionConfig,
    tx: CommandBusTx,
) -> Result<JoinHandle<()>> {
    let mut adapter = HotkeyInputAdapter::new(cfg)?;
    let listener_tx = tx.clone();

    let handle = std::thread::spawn(move || {
        if let Err(err) = listen(move |event: Event| {
            if let Some((key, pressed)) = match event.event_type {
                EventType::KeyPress(key) => Some((key, true)),
                EventType::KeyRelease(key) => Some((key, false)),
                _ => None,
            } {
                if let Some(edge) = adapter.ingest(key, pressed) {
                    if let Some(hotkey_event) = HotkeyInputCoordinator::route(edge) {
                        if let Some(worker_event) = listener_tx.send_hotkey(hotkey_event) {
                            let _ = listener_tx.send_worker(worker_event);
                        }
                    }
                }
            }
        }) {
            log::warn!("hotkey listener ended unexpectedly: {err:?}");
        }
    });

    Ok(handle)
}
