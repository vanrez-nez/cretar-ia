use crate::contracts::commands::RecordingCommand;
use crate::contracts::errors::RecordingErrorCode;
use crate::contracts::events::{HotkeyEvent, PipelineMode, PipelinePhase, RecordingEvent};
use crate::recording::fsm::{transition, NoopReason, RecordedEvent, TransitionResult};
use crate::recording::state::RecordingState;

#[test]
fn idle_press_starts_recording_with_new_session() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);

    let start = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));

    assert_eq!(
        start.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Idle,
            to: PipelinePhase::Starting,
            why: "start_requested",
            command: Some(RecordingCommand::StartRecording),
        }
    );
    assert_eq!(start.next.phase, PipelinePhase::Starting);
    assert_eq!(start.next.session_id, 1);
    assert_eq!(start.next.seq, 1);
}

#[test]
fn push_mode_press_release_stops_recording_once() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);
    let started = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    let started = transition(
        &started.next,
        RecordedEvent::Worker(RecordingEvent::AudioStarted),
    );
    let stopped = transition(&started.next, RecordedEvent::Hotkey(HotkeyEvent::Released));

    assert_eq!(
        stopped.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Recording,
            to: PipelinePhase::Stopping,
            why: "push_release_stop",
            command: Some(RecordingCommand::StopRecording),
        }
    );

    let duplicate = transition(&stopped.next, RecordedEvent::Hotkey(HotkeyEvent::Released));
    assert!(matches!(
        duplicate.result,
        TransitionResult::Noop(NoopReason::InvalidTransition {
            phase: PipelinePhase::Stopping,
            event: "hotkey_released"
        })
    ));
}

#[test]
fn push_mode_release_during_starting_stops_after_audio_started() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);
    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    let pending_stop = transition(&starting.next, RecordedEvent::Hotkey(HotkeyEvent::Released));

    assert_eq!(
        pending_stop.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Starting,
            to: PipelinePhase::Starting,
            why: "push_release_during_start",
            command: None,
        }
    );
    assert!(pending_stop.next.stop_requested_after_start);

    let stopping = transition(
        &pending_stop.next,
        RecordedEvent::Worker(RecordingEvent::AudioStarted),
    );

    assert_eq!(
        stopping.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Starting,
            to: PipelinePhase::Stopping,
            why: "audio_started_stop_requested",
            command: Some(RecordingCommand::StopRecording),
        }
    );
    assert!(!stopping.next.stop_requested_after_start);
}

#[test]
fn toggle_mode_press_toggles_start_and_stop() {
    let state = RecordingState::new(PipelineMode::Toggle, 0);

    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    let recording = transition(
        &starting.next,
        RecordedEvent::Worker(RecordingEvent::AudioStarted),
    );

    let stopping = transition(&recording.next, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    assert_eq!(
        stopping.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Recording,
            to: PipelinePhase::Stopping,
            why: "toggle_press_stop",
            command: Some(RecordingCommand::StopRecording),
        }
    );

    let release_ignored = transition(&stopping.next, RecordedEvent::Hotkey(HotkeyEvent::Released));
    assert!(matches!(
        release_ignored.result,
        TransitionResult::Noop(NoopReason::ToggleReleaseIgnored)
    ));
}

#[test]
fn processing_recovery_and_error_toggle_press_is_noop() {
    let state = RecordingState::new(PipelineMode::Toggle, 0);
    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::TogglePressed));
    let recording = transition(
        &starting.next,
        RecordedEvent::Worker(RecordingEvent::AudioStarted),
    );
    let stopping = transition(
        &recording.next,
        RecordedEvent::Hotkey(HotkeyEvent::TogglePressed),
    );
    let processing = transition(
        &stopping.next,
        RecordedEvent::Worker(RecordingEvent::AudioStopped {
            path: "test.wav".into(),
        }),
    );

    let processing_noop = transition(
        &processing.next,
        RecordedEvent::Hotkey(HotkeyEvent::TogglePressed),
    );
    assert!(matches!(
        processing_noop.result,
        TransitionResult::Noop(NoopReason::TogglePressIgnored),
    ));

    let recovering = transition(
        &processing.next,
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed),
    );
    let recovering_noop = transition(
        &recovering.next,
        RecordedEvent::Hotkey(HotkeyEvent::TogglePressed),
    );
    assert!(matches!(
        recovering_noop.result,
        TransitionResult::Noop(NoopReason::TogglePressIgnored),
    ));

    let error = transition(
        &processing.next,
        RecordedEvent::Worker(RecordingEvent::ProcessFailed {
            code: RecordingErrorCode::Processing,
            reason: "boom".to_string(),
        }),
    );
    let error_noop = transition(
        &error.next,
        RecordedEvent::Hotkey(HotkeyEvent::TogglePressed),
    );
    assert!(matches!(
        error_noop.result,
        TransitionResult::Noop(NoopReason::TogglePressIgnored),
    ));
}

#[test]
fn stopping_success_transitions_to_processing_and_run_command_emitted() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);
    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    let recording = transition(
        &starting.next,
        RecordedEvent::Worker(RecordingEvent::AudioStarted),
    );
    let stopping = transition(
        &recording.next,
        RecordedEvent::Hotkey(HotkeyEvent::Released),
    );
    let processing = transition(
        &stopping.next,
        RecordedEvent::Worker(RecordingEvent::AudioStopped {
            path: "test.wav".into(),
        }),
    );

    assert_eq!(
        processing.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Stopping,
            to: PipelinePhase::Processing,
            why: "audio_stopped",
            command: Some(RecordingCommand::RunProcessing),
        }
    );
    assert_eq!(processing.next.phase, PipelinePhase::Processing);
}

#[test]
fn processing_completed_returns_idle_with_success_path() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);
    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    let recording = transition(
        &starting.next,
        RecordedEvent::Worker(RecordingEvent::AudioStarted),
    );
    let stopping = transition(
        &recording.next,
        RecordedEvent::Hotkey(HotkeyEvent::Released),
    );
    let processing = transition(
        &stopping.next,
        RecordedEvent::Worker(RecordingEvent::AudioStopped {
            path: "test.wav".into(),
        }),
    );
    let completed = transition(
        &processing.next,
        RecordedEvent::Worker(RecordingEvent::ProcessCompleted),
    );

    assert_eq!(
        completed.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Processing,
            to: PipelinePhase::Idle,
            why: "processing_completed",
            command: None,
        }
    );
}

#[test]
fn processing_failed_enters_error_without_cancel_command() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);
    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    let recording = transition(
        &starting.next,
        RecordedEvent::Worker(RecordingEvent::AudioStarted),
    );
    let stopping = transition(
        &recording.next,
        RecordedEvent::Hotkey(HotkeyEvent::Released),
    );
    let processing = transition(
        &stopping.next,
        RecordedEvent::Worker(RecordingEvent::AudioStopped {
            path: "test.wav".into(),
        }),
    );
    let failed = transition(
        &processing.next,
        RecordedEvent::Worker(RecordingEvent::ProcessFailed {
            code: RecordingErrorCode::Processing,
            reason: "provider timeout".to_string(),
        }),
    );

    assert_eq!(
        failed.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Processing,
            to: PipelinePhase::Error,
            why: "processing_failed",
            command: None,
        }
    );
    assert_eq!(failed.next.phase, PipelinePhase::Error);
}

#[test]
fn toggle_mode_release_never_stops_recording() {
    let state = RecordingState::new(PipelineMode::Toggle, 0);
    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::TogglePressed));
    let recording = transition(
        &starting.next,
        RecordedEvent::Worker(RecordingEvent::AudioStarted),
    );

    let released = transition(
        &recording.next,
        RecordedEvent::Hotkey(HotkeyEvent::Released),
    );
    assert!(matches!(
        released.result,
        TransitionResult::Noop(NoopReason::ToggleReleaseIgnored)
    ));

    let pressed_again = transition(&recording.next, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    assert_eq!(
        pressed_again.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Recording,
            to: PipelinePhase::Stopping,
            why: "toggle_press_stop",
            command: Some(RecordingCommand::StopRecording),
        }
    );
}

#[test]
fn cancel_pressed_moves_non_idle_states_to_recovering() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);

    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    let starting_cancel = transition(
        &starting.next,
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed),
    );
    assert_eq!(
        starting_cancel.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Starting,
            to: PipelinePhase::Recovering,
            why: "cancel_pressed",
            command: Some(RecordingCommand::ForceStop),
        }
    );

    let recording = transition(
        &starting.next,
        RecordedEvent::Worker(RecordingEvent::AudioStarted),
    );
    let recording_cancel = transition(
        &recording.next,
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed),
    );
    assert_eq!(
        recording_cancel.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Recording,
            to: PipelinePhase::Recovering,
            why: "cancel_pressed",
            command: Some(RecordingCommand::ForceStop),
        }
    );

    let stopping = transition(
        &recording.next,
        RecordedEvent::Hotkey(HotkeyEvent::Released),
    );
    let stopping_cancel = transition(
        &stopping.next,
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed),
    );
    assert_eq!(
        stopping_cancel.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Stopping,
            to: PipelinePhase::Recovering,
            why: "cancel_pressed",
            command: Some(RecordingCommand::ForceStop),
        }
    );

    let processing = transition(
        &stopping.next,
        RecordedEvent::Worker(RecordingEvent::AudioStopped {
            path: "test.wav".into(),
        }),
    );
    let processing_cancel = transition(
        &processing.next,
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed),
    );
    assert_eq!(
        processing_cancel.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Processing,
            to: PipelinePhase::Recovering,
            why: "cancel_pressed",
            command: Some(RecordingCommand::CancelProcessing),
        }
    );

    let recording_error = transition(
        &processing.next,
        RecordedEvent::Worker(RecordingEvent::ProcessFailed {
            code: crate::contracts::errors::RecordingErrorCode::Processing,
            reason: "processing failed".to_string(),
        }),
    );
    let error_cancel = transition(
        &recording_error.next,
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed),
    );
    assert_eq!(
        error_cancel.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Error,
            to: PipelinePhase::Recovering,
            why: "cancel_pressed",
            command: Some(RecordingCommand::ForceStop),
        }
    );
}

#[test]
fn duplicate_press_during_starting_is_noop() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);
    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));

    let duplicate = transition(&starting.next, RecordedEvent::Hotkey(HotkeyEvent::Pressed));

    assert!(matches!(
        duplicate.result,
        TransitionResult::Noop(NoopReason::InvalidTransition {
            phase: PipelinePhase::Starting,
            event: "hotkey_pressed"
        })
    ));
}

#[test]
fn cancel_during_recovering_is_idempotent() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);
    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    let recovering = transition(
        &starting.next,
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed),
    );

    let duplicate = transition(
        &recovering.next,
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed),
    );

    assert!(matches!(
        duplicate.result,
        TransitionResult::Noop(NoopReason::InvalidTransition {
            phase: PipelinePhase::Recovering,
            event: "hotkey_cancel_pressed",
        })
    ));
}

#[test]
fn recovering_with_recovery_failed_goes_to_error_with_force_stop_hint() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);
    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    let recovering = transition(
        &starting.next,
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed),
    );

    let failed = transition(
        &recovering.next,
        RecordedEvent::Worker(RecordingEvent::RecoveryFailed {
            code: RecordingErrorCode::Unknown,
            reason: "boom".to_string(),
        }),
    );

    assert_eq!(
        failed.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Recovering,
            to: PipelinePhase::Error,
            why: "recovery_failed",
            command: Some(RecordingCommand::ForceStop),
        }
    );
    assert_eq!(failed.next.phase, PipelinePhase::Error);
    assert_eq!(
        failed.next.recovery_hint,
        crate::contracts::errors::RecoveryHint::Manual
    );
}

#[test]
fn recovering_with_recovery_completed_returns_idle() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);
    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    let recovering = transition(
        &starting.next,
        RecordedEvent::Hotkey(HotkeyEvent::CancelPressed),
    );

    let completed = transition(
        &recovering.next,
        RecordedEvent::Worker(RecordingEvent::RecoveryCompleted),
    );

    assert_eq!(
        completed.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Recovering,
            to: PipelinePhase::Idle,
            why: "recovery_completed",
            command: None,
        }
    );
}

#[test]
fn worker_event_in_wrong_phase_is_invalid_noop() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);
    let result = transition(
        &state,
        RecordedEvent::Worker(RecordingEvent::ProcessCompleted),
    );

    assert!(matches!(
        result.result,
        TransitionResult::Noop(NoopReason::InvalidTransition {
            phase: PipelinePhase::Idle,
            event: "process_completed"
        })
    ));
}

#[test]
fn worker_event_carries_recovery_hint_on_error() {
    let mut state = RecordingState::new(PipelineMode::PushToTalk, 0);
    state = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed)).next;
    let start_failed = transition(
        &state,
        RecordedEvent::Worker(RecordingEvent::AudioStartFailed {
            code: RecordingErrorCode::AudioInit,
            reason: "init failed".to_string(),
        }),
    );

    assert_eq!(start_failed.next.phase, PipelinePhase::Error);
    assert_eq!(
        start_failed.next.recovery_hint,
        crate::contracts::errors::RecoveryHint::RetryStart
    );
    assert_eq!(
        start_failed.next.last_reason.as_deref(),
        Some("init failed")
    );
}

#[test]
fn push_press_repeat_is_suppressed_as_autorepeat_noop() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);
    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    assert_eq!(
        starting.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Idle,
            to: PipelinePhase::Starting,
            why: "start_requested",
            command: Some(RecordingCommand::StartRecording),
        }
    );

    let repeat = transition(&starting.next, RecordedEvent::Hotkey(HotkeyEvent::Repeat));
    assert!(matches!(
        repeat.result,
        TransitionResult::Noop(NoopReason::InvalidTransition {
            phase: PipelinePhase::Starting,
            event: "hotkey_repeat"
        })
    ));
}

#[test]
fn toggle_double_press_is_press_toggle_stop_and_ignored() {
    let state = RecordingState::new(PipelineMode::Toggle, 0);
    let started = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    let running = transition(
        &started.next,
        RecordedEvent::Worker(RecordingEvent::AudioStarted),
    );
    assert_eq!(
        running.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Starting,
            to: PipelinePhase::Recording,
            why: "audio_started",
            command: None,
        }
    );

    let second_press = transition(&running.next, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    assert_eq!(
        second_press.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Recording,
            to: PipelinePhase::Stopping,
            why: "toggle_press_stop",
            command: Some(RecordingCommand::StopRecording),
        }
    );

    let ignored = transition(
        &second_press.next,
        RecordedEvent::Hotkey(HotkeyEvent::TogglePressed),
    );
    assert!(matches!(
        ignored.result,
        TransitionResult::Noop(NoopReason::InvalidTransition {
            phase: PipelinePhase::Stopping,
            event: "hotkey_toggle_pressed"
        })
    ));
}

#[test]
fn timeout_from_starting_moves_to_error_with_force_stop() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);
    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));

    let timeout = transition(
        &starting.next,
        RecordedEvent::Worker(RecordingEvent::TimeoutExpired),
    );

    assert_eq!(
        timeout.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Starting,
            to: PipelinePhase::Error,
            why: "starting_timeout",
            command: Some(RecordingCommand::ForceStop),
        }
    );
}

#[test]
fn timeout_from_stopping_moves_to_error_with_force_stop() {
    let state = RecordingState::new(PipelineMode::PushToTalk, 0);
    let starting = transition(&state, RecordedEvent::Hotkey(HotkeyEvent::Pressed));
    let recording = transition(
        &starting.next,
        RecordedEvent::Worker(RecordingEvent::AudioStarted),
    );
    let stopping = transition(
        &recording.next,
        RecordedEvent::Hotkey(HotkeyEvent::Released),
    );

    let timeout = transition(
        &stopping.next,
        RecordedEvent::Worker(RecordingEvent::TimeoutExpired),
    );

    assert_eq!(
        timeout.result,
        TransitionResult::StateChange {
            from: PipelinePhase::Stopping,
            to: PipelinePhase::Error,
            why: "stopping_timeout",
            command: Some(RecordingCommand::ForceStop),
        }
    );
}
