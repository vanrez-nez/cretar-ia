use crate::contracts::commands::RecordingCommand;
use crate::recording::command_bus::CommandBusTx;
use tokio::sync::mpsc;

/// Runtime edge controls accepted from platform adapters such as tray menus and signal handlers.
///
/// Core runtime code should use this boundary type, never legacy `domain::event::AppEvent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeControlEvent {
    Quit,
}

pub fn handle_runtime_control_event(event: RuntimeControlEvent, tx: &CommandBusTx) {
    match event {
        RuntimeControlEvent::Quit => send_command(RecordingCommand::Shutdown, tx),
    }
}

pub fn spawn_runtime_control_bridge(
    mut app_events: mpsc::UnboundedReceiver<RuntimeControlEvent>,
    tx: CommandBusTx,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = app_events.recv().await {
            let should_stop = matches!(event, RuntimeControlEvent::Quit);
            handle_runtime_control_event(event, &tx);
            if should_stop {
                break;
            }
        }
    })
}

fn send_command(command: RecordingCommand, tx: &CommandBusTx) {
    if let Some(event) = tx.send_command(command) {
        let _ = tx.send_worker(event);
    }
}
