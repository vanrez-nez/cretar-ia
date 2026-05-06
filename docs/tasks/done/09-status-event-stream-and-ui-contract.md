# Task 09 — Status event stream and UI contract

## Goal
Replace side-effectful UI logic with one-way status snapshots from orchestrator.

## Architecture decisions
- Tray and cue systems consume the same immutable status stream.
- UI modules do not contain start/stop policy.
- Snapshot emits are guaranteed on transition and on recoverable errors.

## Files to create
- `src-tauri/src/contracts/status.rs` (snapshot struct)
- `src-tauri/src/recording/telemetry.rs` (publisher helpers)

## Files to modify
- `src-tauri/src/tray.rs`
- `src-tauri/src/audio_cues.rs`
- `src-tauri/src/recording/orchestrator.rs`
- `src-tauri/src/runtime/engine.rs`

## Revision details
- Define `SessionStatus` with fields:
  - `pipeline_state`, `error_code`, `error_hint`, `session_id`, `seq`, `uptime_ms`, `last_event`.
- Emit status after every transition and every command failure/saturation event.
- Replace tray/pulse logic with `status_rx` consumption.
- Map status to tray tooltips/cues in a pure renderer table.

## Acceptance checks
- Tray text/icon reflects orchestrator state for all transitions.
- No UI layer can directly call `start/stop` on recorder.
- Status stream is ordered and monotonic per `seq`.
