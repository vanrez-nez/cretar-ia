# Task 07 — Cancel/recovery flow

## Goal
Add a universal cancel path that can be executed from any non-idle state.

## Architecture decisions
- Cancel is always accepted and idempotent.
- Recovery is worker-mediated; orchestrator only observes cleanup outcomes.
- Error state carries recoverability hints and always allows a safe return to idle when cleanup completes.

## Files to create
- `src-tauri/src/recording/workers/recovery.rs`

## Files to modify
- `src-tauri/src/recording/orchestrator.rs`
- `src-tauri/src/recording/workers/audio_worker.rs`
- `src-tauri/src/recording/workers/processor_worker.rs` (abort handling)
- `src-tauri/src/contracts/events.rs` (explicit cancel event)

## Revision details
- Add cancel event and command mapping:
  - `CancelPressed -> ForceStop + optional CancelProcessing`.
- Recovery sequence:
  - transition into `Recovering`,
  - attempt audio worker stop/flush,
  - terminate processing worker if running,
  - emit `RecoveryCompleted` or `RecoveryFailed`.
- Any cleanup failure is non-fatal to coordinator lifecycle but emits `Error` state with hints.

## Acceptance checks
- Cancel succeeds from `Starting`, `Recording`, `Stopping`, and `Processing` with bounded time.
- Consecutive cancels are idempotent.
- No recorder/task handles remain referenced after recovery settles to `Idle`.
