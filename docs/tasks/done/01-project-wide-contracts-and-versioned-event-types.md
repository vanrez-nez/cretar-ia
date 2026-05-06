# Task 01 — Contracts and versioned event types

## Goal
Establish a single, versioned event contract so every component (hotkey input, orchestrator, workers, UI/status) exchanges strongly-typed, traceable messages.

## Architecture decisions
- Contracts live in a dedicated namespace and are the only boundary between layers.
- No `String`-based ad-hoc events; all cross-boundary traffic uses typed enums.
- Use small numeric IDs for traceability (`session_id: u64`, `seq: u64`) to avoid adding a UUID dependency.
- Keep schema version in envelopes (`schema_version: u16`) for forward compatibility without breaking binary startup.

## Files to create
- `src-tauri/src/contracts/mod.rs`
- `src-tauri/src/contracts/events.rs`
- `src-tauri/src/contracts/commands.rs`
- `src-tauri/src/contracts/errors.rs`
- `src-tauri/src/contracts/status.rs`

## Files to modify
- `src-tauri/src/domain.rs` (compatibility bridge only)
- `src-tauri/src/lib.rs` (module exports)

## Revision details
- Define core domain enums:
  - `HotkeyEvent::{Pressed, Released, Repeat, CancelPressed, ModeUpdate, ShutdownRequested}`
  - `RecordingCommand::{StartRecording, StopRecording, ForceStop, RunProcessing, CancelProcessing, Shutdown}`
  - `RecordingEvent::{AudioStarted, AudioStartFailed, AudioStopped, AudioStopFailed, ProcessStarted, ProcessCompleted, ProcessFailed, QueueSaturated, TimeoutExpired, RecoveryCompleted, RecoveryFailed}`
- Define state/meta contracts:
  - `PipelineState` with `phase`, `mode`, `session_id`, `seq`, `last_reason`
  - `SessionStatus` snapshots (`state`, `mode`, `error_code`, `error_hint`, `session_id`, `phase_elapsed_ms`, `source`).
- Define explicit error contract:
  - `RecordingErrorCode::{HotkeyParse, AudioInit, AudioStop, WorkerTimeout, QueueOverflow, Processing, ConfigInvalid, Unknown}`
  - `RecoveryHint::{NoRecovery, RetryProcessing, RetryStart, RetryStop, RetryWorker, Manual}`
- Add `EventEnvelope<T>` generic wrapper with:
  - `session_id`, `seq`, `created_at_ms`, `source`, `schema_version`, and typed `payload`.
- Add helpers:
  - `next_seq()` generator (monotonic in runtime),
  - `new_envelope(...)`,
  - deterministic `Display` for every contract for telemetry.

## Acceptance checks
- Every exported event path in runtime passes through `EventEnvelope`.
- No legacy tuple/string payload crosses hotkey/workers/engine/tray boundary.
- Invalid payload parsing is explicit and returns typed `RecordingErrorCode`.
- Build-time check has zero `TODO` placeholders in contract modules.
