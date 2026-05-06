/// Runtime edge controls accepted from platform adapters such as tray menus and signal handlers.
///
/// Core runtime code should use this boundary type, never legacy `domain::event::AppEvent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeControlEvent {
    Quit,
    ReloadRuntime,
    SwitchInputDevice,
}
