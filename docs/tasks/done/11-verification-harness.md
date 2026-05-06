# Task 11 — Verification harness

## Goal
Prove deterministic behavior of FSM + orchestrator under bursty, failing, and recovery-heavy conditions.

## Architecture decisions
- Unit and integration layers are both required; one without the other is insufficient.
- Verification artifacts live in Rust tests and a markdown runbook for manual hardware validation.

## Files to create
- `src-tauri/src/recording/tests/fsm_tests.rs`
- `src-tauri/src/recording/tests/orchestrator_tests.rs`
- `docs/runbooks/hotkey-pipeline-verification.md`

## Files to modify
- `Cargo.toml` (test dependencies/features if required)
- `src-tauri/src/recording/tests/mod.rs` (test module aggregation)

## Revision details
- Unit scenarios:
  - push press/release normal path,
  - toggle press-press flip,
  - auto-repeat suppression,
  - invalid transition produces `Noop` diagnostic,
  - timeout from starting/stopping transitions to recovery.
- Integration scenarios:
  - cancel during `Starting`, `Recording`, `Stopping`, `Processing`,
  - worker start failure,
  - worker stop failure,
  - queue saturation under rapid events,
  - process failure recovery.
- Runbook scenarios:
  - normal hotkey + mic,
  - denied mic/device,
  - delayed stop,
  - rapid burst with auto-repeat.

## Acceptance checks
- No duplicate start/stop outcomes in deterministic test sequences.
- No leaked recorder handles after repeated create/abort cycles.
- Ordering and snapshot-sequence monotonicity asserted in orchestration tests.
