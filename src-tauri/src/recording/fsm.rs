use crate::contracts::commands::RecordingCommand;
use crate::contracts::errors::{RecoveryHint, RecordingErrorCode};
use crate::contracts::events::{HotkeyEvent, PipelineMode, PipelinePhase, RecordingEvent};
use crate::recording::state::RecordingState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoopReason {
    InvalidTransition {
        phase: PipelinePhase,
        event: &'static str,
    },
    QueueSaturated {
        source: String,
        dropped: u32,
    },
    ModeNoChange,
    ToggleReleaseIgnored,
    TogglePressIgnored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionResult {
    Noop(NoopReason),
    StateChange {
        from: PipelinePhase,
        to: PipelinePhase,
        why: &'static str,
        command: Option<RecordingCommand>,
    },
}

#[derive(Debug, Clone)]
pub enum RecordedEvent {
    Hotkey(HotkeyEvent),
    Worker(RecordingEvent),
}

#[derive(Debug, Clone)]
pub struct Transition {
    pub next: RecordingState,
    pub result: TransitionResult,
}

pub fn transition(state: &RecordingState, event: RecordedEvent) -> Transition {
    let event_name = recorded_event_name(&event);
    let mut next = state.clone();

    let result = match state.phase {
        PipelinePhase::Idle => transition_idle(state, &mut next, event, event_name),
        PipelinePhase::Starting => transition_starting(state, &mut next, event, event_name),
        PipelinePhase::Recording => transition_recording(state, &mut next, event, event_name),
        PipelinePhase::Stopping => transition_stopping(state, &mut next, event, event_name),
        PipelinePhase::Processing => transition_processing(state, &mut next, event, event_name),
        PipelinePhase::Recovering => transition_recovering(state, &mut next, event, event_name),
        PipelinePhase::Error => transition_error(state, &mut next, event, event_name),
    };

    if matches!(result, TransitionResult::Noop(_)) {
        return Transition {
            next: state.clone(),
            result,
        };
    }

    Transition { next, result }
}

fn transition_idle(
    state: &RecordingState,
    next: &mut RecordingState,
    event: RecordedEvent,
    event_name: &'static str,
) -> TransitionResult {
    match event {
        RecordedEvent::Hotkey(HotkeyEvent::Pressed) => {
            *next = state.clone()
                .next_seq()
                .next_session()
                .with_phase(PipelinePhase::Starting)
                .with_recovery_hint(RecoveryHint::NoRecovery)
                .with_reason(None)
                .clear_stop_requested_after_start();
            TransitionResult::StateChange {
                from: PipelinePhase::Idle,
                to: PipelinePhase::Starting,
                why: "start_requested",
                command: Some(RecordingCommand::StartRecording),
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::TogglePressed) if state.mode == PipelineMode::Toggle => {
            *next = state.clone()
                .next_seq()
                .next_session()
                .with_phase(PipelinePhase::Starting)
                .with_recovery_hint(RecoveryHint::NoRecovery)
                .with_reason(None)
                .clear_stop_requested_after_start();
            TransitionResult::StateChange {
                from: PipelinePhase::Idle,
                to: PipelinePhase::Starting,
                why: "start_requested",
                command: Some(RecordingCommand::StartRecording),
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::ModeUpdate(mode)) => {
            if state.mode == mode {
                TransitionResult::Noop(NoopReason::ModeNoChange)
            } else {
                *next = state.clone().with_mode(mode);
                TransitionResult::StateChange {
                    from: PipelinePhase::Idle,
                    to: PipelinePhase::Idle,
                    why: "mode_update",
                    command: None,
                }
            }
        }
        RecordedEvent::Worker(RecordingEvent::QueueSaturated { source, dropped }) => {
            TransitionResult::Noop(NoopReason::QueueSaturated { source, dropped })
        }
        RecordedEvent::Worker(_)
        | RecordedEvent::Hotkey(HotkeyEvent::TogglePressed)
        | RecordedEvent::Hotkey(HotkeyEvent::Released)
        | RecordedEvent::Hotkey(HotkeyEvent::Repeat)
        | RecordedEvent::Hotkey(HotkeyEvent::CancelPressed)
        | RecordedEvent::Hotkey(HotkeyEvent::ShutdownRequested) => {
            TransitionResult::Noop(NoopReason::InvalidTransition {
                phase: PipelinePhase::Idle,
                event: event_name,
            })
        }
    }
}

fn transition_starting(
    state: &RecordingState,
    next: &mut RecordingState,
    event: RecordedEvent,
    event_name: &'static str,
) -> TransitionResult {
    match event {
        RecordedEvent::Worker(RecordingEvent::AudioStarted) => {
            if state.mode == PipelineMode::PushToTalk && state.stop_requested_after_start {
                *next = state.clone()
                    .next_seq()
                    .with_phase(PipelinePhase::Stopping)
                    .with_recovery_hint(RecoveryHint::NoRecovery)
                    .with_reason(None)
                    .clear_stop_requested_after_start();
                TransitionResult::StateChange {
                    from: PipelinePhase::Starting,
                    to: PipelinePhase::Stopping,
                    why: "audio_started_stop_requested",
                    command: Some(RecordingCommand::StopRecording),
                }
            } else {
                *next = state.clone()
                    .next_seq()
                    .with_phase(PipelinePhase::Recording)
                    .with_recovery_hint(RecoveryHint::NoRecovery)
                    .with_reason(None)
                    .clear_stop_requested_after_start();
                TransitionResult::StateChange {
                    from: PipelinePhase::Starting,
                    to: PipelinePhase::Recording,
                    why: "audio_started",
                    command: None,
                }
            }
        }
        RecordedEvent::Worker(RecordingEvent::AudioStartFailed { code, reason }) => {
            *next = state.clone()
                .next_seq()
                .with_error(reason, recovery_hint_for_start(code));
            TransitionResult::StateChange {
                from: PipelinePhase::Starting,
                to: PipelinePhase::Error,
                why: "audio_start_failed",
                command: Some(RecordingCommand::ForceStop),
            }
        }
        RecordedEvent::Worker(RecordingEvent::AudioDeviceUnavailable { code, reason }) => {
            *next = state.clone()
                .next_seq()
                .with_error(reason, recovery_hint_for_start(code));
            TransitionResult::StateChange {
                from: PipelinePhase::Starting,
                to: PipelinePhase::Error,
                why: "audio_device_unavailable",
                command: Some(RecordingCommand::ForceStop),
            }
        }
        RecordedEvent::Worker(RecordingEvent::TimeoutExpired) => {
            *next = state.clone()
                .next_seq()
                .with_error("starting timeout".to_string(), RecoveryHint::RetryWorker);
            TransitionResult::StateChange {
                from: PipelinePhase::Starting,
                to: PipelinePhase::Error,
                why: "starting_timeout",
                command: Some(RecordingCommand::ForceStop),
            }
        }
        RecordedEvent::Worker(RecordingEvent::QueueSaturated { source, dropped }) => {
            TransitionResult::Noop(NoopReason::QueueSaturated { source, dropped })
        }
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed) => {
            *next = state.clone()
                .next_seq()
                .with_recovery("cancel_pressed".to_string(), RecoveryHint::RetryWorker);
            TransitionResult::StateChange {
                from: PipelinePhase::Starting,
                to: PipelinePhase::Recovering,
                why: "cancel_pressed",
                command: Some(RecordingCommand::ForceStop),
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::Released) if state.mode == PipelineMode::PushToTalk => {
            *next = state.clone()
                .next_seq()
                .with_stop_requested_after_start(true);
            TransitionResult::StateChange {
                from: PipelinePhase::Starting,
                to: PipelinePhase::Starting,
                why: "push_release_during_start",
                command: None,
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::ModeUpdate(mode)) => {
            if state.mode == mode {
                TransitionResult::Noop(NoopReason::ModeNoChange)
            } else {
                *next = state.clone()
                    .with_mode(mode)
                    .clear_stop_requested_after_start();
                TransitionResult::StateChange {
                    from: PipelinePhase::Starting,
                    to: PipelinePhase::Starting,
                    why: "mode_update",
                    command: None,
                }
            }
        }
        RecordedEvent::Worker(_)
        | RecordedEvent::Hotkey(HotkeyEvent::Pressed)
        | RecordedEvent::Hotkey(HotkeyEvent::Repeat)
        | RecordedEvent::Hotkey(HotkeyEvent::TogglePressed)
        | RecordedEvent::Hotkey(HotkeyEvent::Released)
        | RecordedEvent::Hotkey(HotkeyEvent::ShutdownRequested) => {
            TransitionResult::Noop(NoopReason::InvalidTransition {
                phase: PipelinePhase::Starting,
                event: event_name,
            })
        }
    }
}

fn transition_recording(
    state: &RecordingState,
    next: &mut RecordingState,
    event: RecordedEvent,
    event_name: &'static str,
) -> TransitionResult {
    match event {
        RecordedEvent::Hotkey(HotkeyEvent::Released) if state.mode == PipelineMode::PushToTalk => {
            *next = state.clone()
                .next_seq()
                .with_phase(PipelinePhase::Stopping)
                .clear_stop_requested_after_start();
            TransitionResult::StateChange {
                from: PipelinePhase::Recording,
                to: PipelinePhase::Stopping,
                why: "push_release_stop",
                command: Some(RecordingCommand::StopRecording),
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::Pressed) if state.mode == PipelineMode::Toggle => {
            *next = state.clone()
                .next_seq()
                .with_phase(PipelinePhase::Stopping)
                .clear_stop_requested_after_start();
            TransitionResult::StateChange {
                from: PipelinePhase::Recording,
                to: PipelinePhase::Stopping,
                why: "toggle_press_stop",
                command: Some(RecordingCommand::StopRecording),
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::TogglePressed) if state.mode == PipelineMode::Toggle => {
            *next = state.clone()
                .next_seq()
                .with_phase(PipelinePhase::Stopping)
                .clear_stop_requested_after_start();
            TransitionResult::StateChange {
                from: PipelinePhase::Recording,
                to: PipelinePhase::Stopping,
                why: "toggle_press_stop",
                command: Some(RecordingCommand::StopRecording),
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::Released) => {
            TransitionResult::Noop(NoopReason::ToggleReleaseIgnored)
        }
        RecordedEvent::Hotkey(HotkeyEvent::Pressed) => {
            TransitionResult::Noop(NoopReason::TogglePressIgnored)
        }
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed) => {
            *next = state.clone()
                .next_seq()
                .with_recovery("cancel_pressed".to_string(), RecoveryHint::RetryStop);
            TransitionResult::StateChange {
                from: PipelinePhase::Recording,
                to: PipelinePhase::Recovering,
                why: "cancel_pressed",
                command: Some(RecordingCommand::ForceStop),
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::ModeUpdate(mode)) => {
            if state.mode == mode {
                TransitionResult::Noop(NoopReason::ModeNoChange)
            } else {
                *next = state.clone().with_mode(mode);
                TransitionResult::StateChange {
                    from: PipelinePhase::Recording,
                    to: PipelinePhase::Recording,
                    why: "mode_update",
                    command: None,
                }
            }
        }
        RecordedEvent::Worker(RecordingEvent::AudioStopped { .. }) => {
            *next = state.clone()
                .next_seq()
                .with_error("unexpected audio stopped".to_string(), RecoveryHint::RetryWorker);
            TransitionResult::StateChange {
                from: PipelinePhase::Recording,
                to: PipelinePhase::Error,
                why: "unexpected_audio_stopped",
                command: Some(RecordingCommand::ForceStop),
            }
        }
        RecordedEvent::Worker(RecordingEvent::AudioDeviceUnavailable { code, reason }) => {
            *next = state.clone()
                .next_seq()
                .with_error(reason, recovery_hint_for_start(code));
            TransitionResult::StateChange {
                from: PipelinePhase::Recording,
                to: PipelinePhase::Error,
                why: "audio_device_unavailable",
                command: Some(RecordingCommand::ForceStop),
            }
        }
        RecordedEvent::Worker(RecordingEvent::TimeoutExpired) => {
            *next = state.clone()
                .next_seq()
                .with_error("recording timeout".to_string(), RecoveryHint::RetryWorker);
            TransitionResult::StateChange {
                from: PipelinePhase::Recording,
                to: PipelinePhase::Error,
                why: "timeout",
                command: Some(RecordingCommand::ForceStop),
            }
        }
        RecordedEvent::Worker(RecordingEvent::QueueSaturated { source, dropped }) => {
            TransitionResult::Noop(NoopReason::QueueSaturated { source, dropped })
        }
        RecordedEvent::Worker(_)
        | RecordedEvent::Hotkey(HotkeyEvent::Repeat)
        | RecordedEvent::Hotkey(HotkeyEvent::TogglePressed)
        | RecordedEvent::Hotkey(HotkeyEvent::ShutdownRequested) => {
            TransitionResult::Noop(NoopReason::InvalidTransition {
                phase: PipelinePhase::Recording,
                event: event_name,
            })
        }
    }
}

fn transition_stopping(
    state: &RecordingState,
    next: &mut RecordingState,
    event: RecordedEvent,
    event_name: &'static str,
) -> TransitionResult {
    match event {
        RecordedEvent::Worker(RecordingEvent::AudioStopped { .. }) => {
            *next = state.clone()
                .next_seq()
                .with_phase(PipelinePhase::Processing)
                .clear_stop_requested_after_start();
            TransitionResult::StateChange {
                from: PipelinePhase::Stopping,
                to: PipelinePhase::Processing,
                why: "audio_stopped",
                command: Some(RecordingCommand::RunProcessing),
            }
        }
        RecordedEvent::Worker(RecordingEvent::AudioStopFailed { code, reason }) => {
            *next = state.clone()
                .next_seq()
                .with_error(reason, recovery_hint_for_stop(code));
            TransitionResult::StateChange {
                from: PipelinePhase::Stopping,
                to: PipelinePhase::Error,
                why: "audio_stop_failed",
                command: Some(RecordingCommand::ForceStop),
            }
        }
        RecordedEvent::Worker(RecordingEvent::TimeoutExpired) => {
            *next = state.clone()
                .next_seq()
                .with_error("stopping timeout".to_string(), RecoveryHint::RetryWorker);
            TransitionResult::StateChange {
                from: PipelinePhase::Stopping,
                to: PipelinePhase::Error,
                why: "stopping_timeout",
                command: Some(RecordingCommand::ForceStop),
            }
        }
        RecordedEvent::Worker(RecordingEvent::QueueSaturated { source, dropped }) => {
            TransitionResult::Noop(NoopReason::QueueSaturated { source, dropped })
        }
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed) => {
            *next = state.clone()
                .next_seq()
                .with_recovery("cancel_pressed".to_string(), RecoveryHint::RetryStop);
            TransitionResult::StateChange {
                from: PipelinePhase::Stopping,
                to: PipelinePhase::Recovering,
                why: "cancel_pressed",
                command: Some(RecordingCommand::ForceStop),
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::ModeUpdate(mode)) => {
            if state.mode == mode {
                TransitionResult::Noop(NoopReason::ModeNoChange)
            } else {
                *next = state.clone().with_mode(mode);
                TransitionResult::StateChange {
                    from: PipelinePhase::Stopping,
                    to: PipelinePhase::Stopping,
                    why: "mode_update",
                    command: None,
                }
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::Released) if state.mode == PipelineMode::Toggle => {
            TransitionResult::Noop(NoopReason::ToggleReleaseIgnored)
        }
        RecordedEvent::Hotkey(HotkeyEvent::Pressed)
        | RecordedEvent::Hotkey(HotkeyEvent::Released)
        | RecordedEvent::Hotkey(HotkeyEvent::Repeat)
        | RecordedEvent::Hotkey(HotkeyEvent::TogglePressed)
        | RecordedEvent::Hotkey(HotkeyEvent::ShutdownRequested) => {
            TransitionResult::Noop(NoopReason::InvalidTransition {
                phase: PipelinePhase::Stopping,
                event: event_name,
            })
        }
        RecordedEvent::Worker(_) => TransitionResult::Noop(NoopReason::InvalidTransition {
            phase: PipelinePhase::Stopping,
            event: event_name,
        }),
    }
}

fn transition_processing(
    state: &RecordingState,
    next: &mut RecordingState,
    event: RecordedEvent,
    event_name: &'static str,
) -> TransitionResult {
    match event {
        RecordedEvent::Worker(RecordingEvent::ProcessCompleted) => {
            *next = state.clone()
                .next_seq()
                .with_phase(PipelinePhase::Idle)
                .with_recovery_hint(RecoveryHint::NoRecovery)
                .with_reason(None)
                .clear_stop_requested_after_start();
            TransitionResult::StateChange {
                from: PipelinePhase::Processing,
                to: PipelinePhase::Idle,
                why: "processing_completed",
                command: None,
            }
        }
        RecordedEvent::Worker(RecordingEvent::ProcessFailed { code, reason }) => {
            let recovery_hint = if is_output_injection_error(&reason) {
                RecoveryHint::Manual
            } else {
                recovery_hint_for_processing(code)
            };
            *next = state.clone()
                .next_seq()
                .with_error(reason, recovery_hint);
            TransitionResult::StateChange {
                from: PipelinePhase::Processing,
                to: PipelinePhase::Error,
                why: "processing_failed",
                command: None,
            }
        }
        RecordedEvent::Worker(RecordingEvent::TimeoutExpired) => {
            *next = state.clone()
                .next_seq()
                .with_error("processing timeout".to_string(), RecoveryHint::RetryWorker);
            TransitionResult::StateChange {
                from: PipelinePhase::Processing,
                to: PipelinePhase::Error,
                why: "processing_timeout",
                command: Some(RecordingCommand::CancelProcessing),
            }
        }
        RecordedEvent::Worker(RecordingEvent::QueueSaturated { source, dropped }) => {
            TransitionResult::Noop(NoopReason::QueueSaturated { source, dropped })
        }
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed) => {
            *next = state.clone()
                .next_seq()
                .with_recovery("cancel_processing".to_string(), RecoveryHint::RetryProcessing);
            TransitionResult::StateChange {
                from: PipelinePhase::Processing,
                to: PipelinePhase::Recovering,
                why: "cancel_pressed",
                command: Some(RecordingCommand::CancelProcessing),
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::ModeUpdate(mode)) => {
            if state.mode == mode {
                TransitionResult::Noop(NoopReason::ModeNoChange)
            } else {
                *next = state.clone().with_mode(mode);
                TransitionResult::StateChange {
                    from: PipelinePhase::Processing,
                    to: PipelinePhase::Processing,
                    why: "mode_update",
                    command: None,
                }
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::TogglePressed) => {
            TransitionResult::Noop(NoopReason::TogglePressIgnored)
        }
        RecordedEvent::Hotkey(HotkeyEvent::Pressed) if state.mode == PipelineMode::Toggle => {
            TransitionResult::Noop(NoopReason::TogglePressIgnored)
        }
        RecordedEvent::Hotkey(HotkeyEvent::Pressed)
        | RecordedEvent::Hotkey(HotkeyEvent::Released)
        | RecordedEvent::Hotkey(HotkeyEvent::Repeat)
        | RecordedEvent::Hotkey(HotkeyEvent::ShutdownRequested)
        | RecordedEvent::Worker(_) => {
            TransitionResult::Noop(NoopReason::InvalidTransition {
                phase: PipelinePhase::Processing,
                event: event_name,
            })
        }
    }
}

fn transition_recovering(
    state: &RecordingState,
    next: &mut RecordingState,
    event: RecordedEvent,
    event_name: &'static str,
) -> TransitionResult {
    match event {
        RecordedEvent::Worker(RecordingEvent::RecoveryCompleted) => {
            *next = state.clone()
                .next_seq()
                .with_phase(PipelinePhase::Idle)
                .with_recovery_hint(RecoveryHint::NoRecovery)
                .with_reason(None)
                .clear_stop_requested_after_start();
            TransitionResult::StateChange {
                from: PipelinePhase::Recovering,
                to: PipelinePhase::Idle,
                why: "recovery_completed",
                command: None,
            }
        }
        RecordedEvent::Worker(RecordingEvent::RecoveryFailed { code, reason }) => {
            *next = state.clone()
                .next_seq()
                .with_error(reason, recovery_hint_for_error(code));
            TransitionResult::StateChange {
                from: PipelinePhase::Recovering,
                to: PipelinePhase::Error,
                why: "recovery_failed",
                command: Some(RecordingCommand::ForceStop),
            }
        }
        RecordedEvent::Worker(RecordingEvent::QueueSaturated { source, dropped }) => {
            TransitionResult::Noop(NoopReason::QueueSaturated { source, dropped })
        }
        RecordedEvent::Hotkey(HotkeyEvent::ModeUpdate(mode)) => {
            if state.mode == mode {
                TransitionResult::Noop(NoopReason::ModeNoChange)
            } else {
                *next = state.clone().with_mode(mode);
                TransitionResult::StateChange {
                    from: PipelinePhase::Recovering,
                    to: PipelinePhase::Recovering,
                    why: "mode_update",
                    command: None,
                }
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::TogglePressed) => {
            TransitionResult::Noop(NoopReason::TogglePressIgnored)
        }
        RecordedEvent::Hotkey(HotkeyEvent::Pressed) if state.mode == PipelineMode::Toggle => {
            TransitionResult::Noop(NoopReason::TogglePressIgnored)
        }
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed)
        | RecordedEvent::Hotkey(HotkeyEvent::Pressed)
        | RecordedEvent::Hotkey(HotkeyEvent::Released)
        | RecordedEvent::Hotkey(HotkeyEvent::Repeat)
        | RecordedEvent::Hotkey(HotkeyEvent::ShutdownRequested)
        | RecordedEvent::Worker(_) => TransitionResult::Noop(NoopReason::InvalidTransition {
            phase: PipelinePhase::Recovering,
            event: event_name,
        }),
    }
}

fn transition_error(
    state: &RecordingState,
    next: &mut RecordingState,
    event: RecordedEvent,
    event_name: &'static str,
) -> TransitionResult {
    match event {
        RecordedEvent::Worker(RecordingEvent::RecoveryCompleted) => {
            *next = state.clone()
                .next_seq()
                .with_phase(PipelinePhase::Idle)
                .with_recovery_hint(RecoveryHint::NoRecovery)
                .with_reason(None)
                .clear_stop_requested_after_start();
            TransitionResult::StateChange {
                from: PipelinePhase::Error,
                to: PipelinePhase::Idle,
                why: "recovery_completed",
                command: None,
            }
        }
        RecordedEvent::Worker(RecordingEvent::RecoveryFailed { code, reason }) => {
            *next = state.clone()
                .next_seq()
                .with_error(reason, recovery_hint_for_error(code));
            TransitionResult::StateChange {
                from: PipelinePhase::Error,
                to: PipelinePhase::Error,
                why: "recovery_failed",
                command: Some(RecordingCommand::ForceStop),
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed) => {
            *next = state.clone()
                .next_seq()
                .with_recovery("cancel_pressed".to_string(), state.recovery_hint);
            TransitionResult::StateChange {
                from: PipelinePhase::Error,
                to: PipelinePhase::Recovering,
                why: "cancel_pressed",
                command: Some(RecordingCommand::ForceStop),
            }
        }
        RecordedEvent::Worker(RecordingEvent::QueueSaturated { source, dropped }) => {
            TransitionResult::Noop(NoopReason::QueueSaturated { source, dropped })
        }
        RecordedEvent::Hotkey(HotkeyEvent::ModeUpdate(mode)) => {
            if state.mode == mode {
                TransitionResult::Noop(NoopReason::ModeNoChange)
            } else {
                *next = state.clone().with_mode(mode);
                TransitionResult::StateChange {
                    from: PipelinePhase::Error,
                    to: PipelinePhase::Error,
                    why: "mode_update",
                    command: None,
                }
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::Pressed) => {
            *next = state.clone()
                .next_seq()
                .next_session()
                .with_phase(PipelinePhase::Starting)
                .with_recovery_hint(RecoveryHint::NoRecovery)
                .with_reason(None)
                .clear_stop_requested_after_start();
            TransitionResult::StateChange {
                from: PipelinePhase::Error,
                to: PipelinePhase::Starting,
                why: "start_requested",
                command: Some(RecordingCommand::StartRecording),
            }
        }
        RecordedEvent::Hotkey(HotkeyEvent::TogglePressed) => {
            TransitionResult::Noop(NoopReason::TogglePressIgnored)
        }
        RecordedEvent::Worker(_)
        | RecordedEvent::Hotkey(HotkeyEvent::Released)
        | RecordedEvent::Hotkey(HotkeyEvent::Repeat)
        | RecordedEvent::Hotkey(HotkeyEvent::ShutdownRequested) => {
            TransitionResult::Noop(NoopReason::InvalidTransition {
                phase: PipelinePhase::Error,
                event: event_name,
            })
        }
    }
}

fn recorded_event_name(event: &RecordedEvent) -> &'static str {
    match event {
        RecordedEvent::Hotkey(HotkeyEvent::Pressed) => "hotkey_pressed",
        RecordedEvent::Hotkey(HotkeyEvent::TogglePressed) => "hotkey_toggle_pressed",
        RecordedEvent::Hotkey(HotkeyEvent::Released) => "hotkey_released",
        RecordedEvent::Hotkey(HotkeyEvent::Repeat) => "hotkey_repeat",
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed) => "hotkey_cancel_pressed",
        RecordedEvent::Hotkey(HotkeyEvent::ModeUpdate(_)) => "hotkey_mode_update",
        RecordedEvent::Hotkey(HotkeyEvent::ShutdownRequested) => "hotkey_shutdown_requested",
        RecordedEvent::Worker(RecordingEvent::AudioStarted) => "audio_started",
        RecordedEvent::Worker(RecordingEvent::AudioStartFailed { .. }) => "audio_start_failed",
        RecordedEvent::Worker(RecordingEvent::AudioStopped { .. }) => "audio_stopped",
        RecordedEvent::Worker(RecordingEvent::AudioStopFailed { .. }) => "audio_stop_failed",
        RecordedEvent::Worker(RecordingEvent::AudioDeviceUnavailable { .. }) => "audio_device_unavailable",
        RecordedEvent::Worker(RecordingEvent::ProcessStarted) => "process_started",
        RecordedEvent::Worker(RecordingEvent::ProcessCompleted) => "process_completed",
        RecordedEvent::Worker(RecordingEvent::ProcessFailed { .. }) => "process_failed",
        RecordedEvent::Worker(RecordingEvent::TransformFailed { .. }) => "transform_failed",
        RecordedEvent::Worker(RecordingEvent::QueueSaturated { .. }) => "queue_saturated",
        RecordedEvent::Worker(RecordingEvent::TimeoutExpired) => "timeout_expired",
        RecordedEvent::Worker(RecordingEvent::RecoveryCompleted) => "recovery_completed",
        RecordedEvent::Worker(RecordingEvent::RecoveryFailed { .. }) => "recovery_failed",
    }
}

fn is_output_injection_error(reason: &str) -> bool {
    reason.starts_with("inject error:")
}

fn recovery_hint_for_start(code: RecordingErrorCode) -> RecoveryHint {
    match code {
        RecordingErrorCode::HotkeyParse => RecoveryHint::Manual,
        RecordingErrorCode::AudioInit => RecoveryHint::RetryStart,
        RecordingErrorCode::ConfigInvalid => RecoveryHint::Manual,
        RecordingErrorCode::WorkerTimeout => RecoveryHint::RetryWorker,
        RecordingErrorCode::QueueOverflow => RecoveryHint::RetryWorker,
        RecordingErrorCode::Unknown => RecoveryHint::RetryStart,
        RecordingErrorCode::AudioStop | RecordingErrorCode::Processing => RecoveryHint::RetryStart,
    }
}

fn recovery_hint_for_stop(code: RecordingErrorCode) -> RecoveryHint {
    match code {
        RecordingErrorCode::AudioStop => RecoveryHint::RetryStop,
        RecordingErrorCode::HotkeyParse => RecoveryHint::Manual,
        RecordingErrorCode::WorkerTimeout => RecoveryHint::RetryWorker,
        RecordingErrorCode::QueueOverflow => RecoveryHint::RetryWorker,
        RecordingErrorCode::ConfigInvalid => RecoveryHint::Manual,
        RecordingErrorCode::AudioInit => RecoveryHint::RetryStart,
        RecordingErrorCode::Processing => RecoveryHint::RetryStop,
        RecordingErrorCode::Unknown => RecoveryHint::RetryStop,
    }
}

fn recovery_hint_for_processing(code: RecordingErrorCode) -> RecoveryHint {
    match code {
        RecordingErrorCode::Processing => RecoveryHint::RetryProcessing,
        RecordingErrorCode::HotkeyParse => RecoveryHint::Manual,
        RecordingErrorCode::WorkerTimeout => RecoveryHint::RetryWorker,
        RecordingErrorCode::QueueOverflow => RecoveryHint::RetryWorker,
        RecordingErrorCode::AudioStop => RecoveryHint::RetryProcessing,
        RecordingErrorCode::ConfigInvalid => RecoveryHint::Manual,
        RecordingErrorCode::AudioInit => RecoveryHint::RetryStart,
        RecordingErrorCode::Unknown => RecoveryHint::RetryProcessing,
    }
}

fn recovery_hint_for_error(code: RecordingErrorCode) -> RecoveryHint {
    match code {
        RecordingErrorCode::AudioStop => RecoveryHint::RetryStop,
        RecordingErrorCode::AudioInit => RecoveryHint::RetryStart,
        RecordingErrorCode::Processing => RecoveryHint::RetryProcessing,
        RecordingErrorCode::WorkerTimeout => RecoveryHint::RetryWorker,
        RecordingErrorCode::QueueOverflow => RecoveryHint::RetryWorker,
        RecordingErrorCode::HotkeyParse | RecordingErrorCode::ConfigInvalid => RecoveryHint::Manual,
        RecordingErrorCode::Unknown => RecoveryHint::Manual,
    }
}
