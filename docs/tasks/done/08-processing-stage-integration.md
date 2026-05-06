# Task 08 — Processing stage integration

## Goal
Make post-stop transcription/Delivery an explicit FSM stage with resilient worker outcomes.

## Architecture decisions
- Processing has explicit command lifecycle and failure outcomes.
- Engine never invokes transcription directly anymore.
- Processing failure remains recoverable with explicit hints and user-visible state.

## Files to create
- `src-tauri/src/recording/workers/processor_worker.rs`

## Files to modify
- `src-tauri/src/openrouter.rs` (used by worker through contract)
- `src-tauri/src/inject.rs` (called by worker)
- `src-tauri/src/recording/orchestrator.rs`
- `src-tauri/src/recording/fsm.rs`
- `src-tauri/src/config.rs` (timeouts or retry knobs)

## Revision details
- Add worker command mapping:
  - `RunProcessing` -> transcribe then deliver output,
  - `CancelProcessing` -> abort if supported/feasible,
  - `RetryProcessing` optional path using preserved audio artifact.
- On stop success, emit `ProcessStarted` and enter `Processing` before command dispatch.
- On process completion, emit `ProcessCompleted` and return to `Idle`.
- On process failure, emit `ProcessFailed` with `RecordingErrorCode::Processing` and hints.
- Artifact policy: keep WAV in recordings directory by default, with cleanup policy configurable by config.

## Acceptance checks
- A stop event cannot skip processing completion phase.
- Processing failure keeps the last artifact and transitions to recoverable state.
- Processing result errors are visible in status stream and tray output.
