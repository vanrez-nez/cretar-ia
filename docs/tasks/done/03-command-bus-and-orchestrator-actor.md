# Task 03 — Command bus and orchestrator actor

## Goal
Create a single orchestrator task that owns FSM state and serializes all side effects.

## Architecture decisions
- Bounded queues are mandatory; backpressure is observable and testable.
- Orchestrator is the only task that owns mutable FSM state.
- Worker events are merged into a single internal event channel to keep ordering.
- Shutdown is tokenized and idempotent.

## Files to create
- `src-tauri/src/recording/command_bus.rs`
- `src-tauri/src/recording/orchestrator.rs`
- `src-tauri/src/recording/telemetry.rs`

## Files to modify
- `src-tauri/src/recording/mod.rs`
- `src-tauri/src/runtime/engine.rs`
- `src-tauri/src/hotkey.rs` (event source routing only)

## Revision details
- Implement `Orchestrator` with:
  - bounded `hotkey_rx` + `worker_rx`,
  - bounded internal command queue,
  - `start()` that spawns worker tasks,
  - `run()` loop driven by `tokio::select!`.
- Introduce queue capacity constants and queue diagnostics:
  - `HOTKEY_QUEUE_CAPACITY`, `WORKER_QUEUE_CAPACITY`, both config overrideable.
- Timeout model:
  - settling timers for `Starting` and `Stopping` state.
  - emit `RecordingEvent::TimeoutExpired` on expiry.
- Backpressure behavior:
  - If producer overflows, emit `QueueSaturated` with source and dropped count.
- Cancellation and shutdown tokens:
  - cancel path always safe when no handle exists.

## Acceptance checks
- No module outside orchestrator invokes worker start/stop/process directly.
- Event and command ordering is stable under burst.
- A queued overflow event does not crash, and state remains safe.
