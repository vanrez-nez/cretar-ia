use crate::contracts::commands::RecordingCommand;
use crate::contracts::events::HotkeyEvent;
use crate::domain::AppEvent;
use crate::recording::command_bus::CommandBusTx;
use tokio::sync::mpsc;

/// Compatibility bridge for external/runtime-adjacent legacy callers that still emit `AppEvent`.
///
/// Legacy callers should migrate to contract inputs (`HotkeyEvent` / `RecordingCommand`) as soon as
/// possible, but this adapter keeps compatibility without giving them direct control over
/// orchestrator internals.
pub fn handle_legacy_app_event(event: AppEvent, tx: &CommandBusTx) {
    match event {
        AppEvent::Start => send_command(RecordingCommand::StartRecording, tx),
        AppEvent::Stop => send_command(RecordingCommand::StopRecording, tx),
        AppEvent::Toggle => send_hotkey(HotkeyEvent::TogglePressed, tx),
        AppEvent::Quit => send_command(RecordingCommand::Shutdown, tx),
    }
}

pub fn spawn_legacy_app_event_bridge(
    mut app_events: mpsc::UnboundedReceiver<AppEvent>,
    tx: CommandBusTx,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = app_events.recv().await {
            handle_legacy_app_event(event, &tx);
        }
    })
}

fn send_command(command: RecordingCommand, tx: &CommandBusTx) {
    if let Some(event) = tx.send_command(command) {
        let _ = tx.send_worker(event);
    }
}

fn send_hotkey(event: HotkeyEvent, tx: &CommandBusTx) {
    if let Some(event) = tx.send_hotkey(event) {
        let _ = tx.send_worker(event);
    }
}
