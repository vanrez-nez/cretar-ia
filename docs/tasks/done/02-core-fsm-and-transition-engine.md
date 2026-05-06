# Task 02 — Core FSM and transition engine

## Goal
Implement a deterministic state machine that owns all recording decision logic and transition semantics.

## Architecture decisions
- FSM is pure: input events + context only, no side effects.
- Every `(state, event)` pair has an explicit outcome.
- Invalid transitions are first-class diagnostics, never silent no-ops.
- Add `Recovering` as an explicit repair state to preserve determinism under partial failures.

## Files to create
- `src-tauri/src/recording/fsm.rs`
- `src-tauri/src/recording/state.rs`

## Files to modify
- `src-tauri/src/recording/mod.rs` (module wiring)
- `src-tauri/src/recording/tests/fsm_tests.rs` (unit tests)

## Revision details
- Define states as:
  - `Idle`, `Starting`, `Recording`, `Stopping`, `Processing`, `Recovering`, `Error`.
- Define transition result type:
  - `TransitionResult::{Noop(NoopReason), EmitCommand(RecordingCommand), StateChange{from,to,why,command})`.
- Implement matrix with explicit guards for:
  - push press/release policy,
  - toggle press policy,
  - cancel policy,
  - settle timeout transitions,
  - queue saturation handling.
- Invariants enforced in FSM layer:
  - single active session ID,
  - terminal errors always carry `RecoveryHint`,
  - worker events in wrong state generate `Noop` + diagnostic.
- Keep `mode` in state snapshot (PushToTalk/Toggle) so transitions are mode-aware without querying config each time.

## Acceptance checks
- The transition function is total for all event/state combos.
- All invalid transitions return `Noop(NoopReason::InvalidTransition)` with stable reason code.
- Tests prove no duplicate start/stop commands are produced from one physical key stroke sequence.
