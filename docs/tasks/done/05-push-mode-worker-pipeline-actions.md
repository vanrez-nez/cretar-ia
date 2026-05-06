# Task 05 — Push-mode worker pipeline actions

## Goal
Move push-mode start/stop recording into dedicated workers with typed completion outcomes.

## Architecture decisions
- `audio::Recorder` remains the low-level owner of capture stream but is used only from worker code.
- Start/stop are async-friendly wrappers over the existing sync recorder methods.
- Worker outcomes are authoritative for FSM transitions.

## Files to create
- `src-tauri/src/recording/workers/audio_worker.rs`

## Files to modify
- `src-tauri/src/audio.rs` (expose minimal worker-safe API hooks)
- `src-tauri/src/recording/orchestrator.rs`
- `src-tauri/src/recording/command_bus.rs`
- `src-tauri/src/config.rs` (if capture timing tuning affects worker behavior)

## Revision details
- Implement worker request handler for:
  - `StartRecording` -> initialize recorder context and return `AudioStarted`/`AudioStartFailed`.
  - `StopRecording` -> finalise recorder, emit `AudioStopped(path)` or `AudioStopFailed`.
  - `ForceStop` -> stop if active; ignore if absent.
- Preserve existing WAV writing path and metadata generation from `audio::Recorder::start`/`stop`.
- Ensure stop is cancellable and idempotent (calling stop multiple times only first call has effect).

## Acceptance checks
- No UI/hotkey path blocks on stream start or stop.
- Worker returns explicit failures with actionable `RecordingErrorCode`.
- Start-stop burst cannot produce overlapping active recorder handles.
