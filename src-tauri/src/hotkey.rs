mod input;

pub use input::listener::start_listener;
pub use input::traits::{HotkeyEdgeEvent, HotkeyInputCoordinator, HotkeyInputListener, HotkeyInputProvider};

use crate::config::InteractionConfig;
use crate::recording::command_bus::CommandBusTx;
use anyhow::Result;

pub fn spawn_listener(
    cfg: InteractionConfig,
    tx: CommandBusTx,
) -> Result<std::thread::JoinHandle<()>> {
    // Compatibility entrypoint: emits `HotkeyEvent`s through `CommandBusTx`, never direct recorder calls.
    <crate::hotkey::input::listener::RdevHotkeyListener as HotkeyInputListener>::start(cfg, tx)
}
