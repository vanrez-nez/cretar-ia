# Task 14 — Session status stream backpressure and bounded publishing

## Goal
Avoid unbounded memory growth from status/event publishing by replacing high-volume unbounded channels with bounded + overflow policy while preserving UI/tray compatibility.

## Architecture decisions
- Keep current `SessionStatus` shape and consumers.
- Add bounded channel capacity and explicit overflow handling policy (drop-oldest or coalesce-last) that is explicit in code comments and tests.
- Avoid adding a new event bus or third-party stream layer.

## Files to create
- None.

## Files to modify
- `src-tauri/src/recording/orchestrator.rs`
- `src-tauri/src/contracts/status.rs`
- `src-tauri/src/recording/tests/orchestrator_tests.rs`

## Revision details
1. Replace unbounded status publish path in orchestrator initialization and status emitters with bounded channel capacity.
2. Define and document overflow behavior where publish can outpace consumer.
3. Ensure status observers still receive a coherent latest state for UI/state display.
4. Add/adjust tests for saturation and continuity when events are dropped/merged under pressure.

## Acceptance checks
- Status event producer cannot grow memory unbounded when consumers are slow.
- UI-facing status remains coherent and monotonic in source-consistent order.
- Overflow behavior is deterministic and covered by tests.
