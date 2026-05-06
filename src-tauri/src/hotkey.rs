mod input;

pub(crate) use input::validate_shortcut;

use crate::config::InteractionConfig;
use crate::recording::command_bus::CommandBusTx;
use anyhow::Result;
use input::traits::HotkeyInputListener;

pub fn spawn_listener(
    cfg: InteractionConfig,
    tx: CommandBusTx,
) -> Result<std::thread::JoinHandle<()>> {
    // Compatibility entrypoint: emits `HotkeyEvent`s through `CommandBusTx`, never direct recorder calls.
    <crate::hotkey::input::listener::RdevHotkeyListener as HotkeyInputListener>::start(cfg, tx)
}
