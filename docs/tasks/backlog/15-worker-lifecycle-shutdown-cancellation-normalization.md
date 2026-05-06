# Task 15 — Normalize worker shutdown and cancellation semantics

## Goal
Simplify worker lifecycle behavior for stop/abort cases with deterministic cancellation, better channel closure semantics, and clearer teardown ownership.

## Architecture decisions
- Keep current Tokio + blocking worker split.
- Make lifecycle transitions explicit: `running -> stopping -> stopped` with idempotent stop handling.
- Avoid adding a new cancellation framework; prefer existing `Command` flow and ordered event publishing.

## Files to create
- None.

## Files to modify
- `src-tauri/src/recording/workers/audio_worker.rs`
- `src-tauri/src/recording/workers/processor_worker.rs`
- `src-tauri/src/recording/command_bus.rs`
- `src-tauri/src/recording/orchestrator.rs`

## Revision details
1. Normalize command acknowledgements for shutdown paths.
2. Close or drain command channels predictably before awaiting worker task exit.
3. Guard race windows where stop commands and stop-source transitions can interleave.
4. Keep behavior of existing states/events but make stop/cancel outcomes explicit and bounded in duration.

## Acceptance checks
- Repeated stop requests do not deadlock or panic.
- Forced stop path completes in bounded time for blocked workers where practical.
- No loss of existing recovery semantics for existing tests.
