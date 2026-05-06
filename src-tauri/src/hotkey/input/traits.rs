use crate::config::InteractionConfig;
use crate::contracts::events::HotkeyEvent;
use crate::recording::command_bus::CommandBusTx;
use anyhow::Result;
use std::thread::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEdgeEvent {
    Pressed,
    TogglePressed,
    Released,
    Repeat,
    Ignored,
}

pub trait HotkeyInputProvider {
    fn route(edge: HotkeyEdgeEvent) -> Option<HotkeyEvent>;
}

pub trait HotkeyInputListener {
    fn start(cfg: InteractionConfig, tx: CommandBusTx) -> Result<JoinHandle<()>>;
}

pub struct HotkeyInputCoordinator;

impl HotkeyInputCoordinator {
    pub fn route(edge: HotkeyEdgeEvent) -> Option<HotkeyEvent> {
        <Self as HotkeyInputProvider>::route(edge)
    }
}

impl HotkeyInputProvider for HotkeyInputCoordinator {
    fn route(edge: HotkeyEdgeEvent) -> Option<HotkeyEvent> {
        match edge {
            HotkeyEdgeEvent::Pressed => Some(HotkeyEvent::Pressed),
            HotkeyEdgeEvent::TogglePressed => Some(HotkeyEvent::TogglePressed),
            HotkeyEdgeEvent::Released => Some(HotkeyEvent::Released),
            HotkeyEdgeEvent::Repeat => None,
            HotkeyEdgeEvent::Ignored => None,
        }
    }
}
