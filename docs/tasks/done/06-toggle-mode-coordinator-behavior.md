# Task 06 — Toggle-mode coordinator behavior

## Goal
Implement deterministic toggle-mode policy inside FSM/orchestrator, independent of listener shape.

## Architecture decisions
- Toggle behavior is event-driven and press-only.
- Release policy is ignored in toggle mode unless recovery guard requires it.
- Reconfiguration of mode is transactional and applied at next event boundary.

## Files to create/modify
- `src-tauri/src/recording/fsm.rs` (toggle transition rules)
- `src-tauri/src/hotkey/input/listener.rs` (edge labeling only)
- `src-tauri/src/config.rs` (mode change propagation)
- `src-tauri/src/recording/orchestrator.rs` (mode snapshot updates)

## Revision details
- Add `ToggleModePressed` event path from input mapping.
- Implement transition rules:
  - `Idle + TogglePress -> Starting` and dispatch `StartRecording`.
  - `Recording + TogglePress -> Stopping` and dispatch `StopRecording`.
  - `Processing/Recovering/Error + TogglePress -> Noop + hint`.
- Add mode snapshot in runtime context to avoid reading config on every input event.
- On mode config change event, update orchestrator snapshot and publish status event.

## Acceptance checks
- Toggle press sequence `down/up/down/up` behaves as expected under timing noise.
- Toggle release never triggers stop.
- Mode change mid-session does not alter active command currently in progress.
