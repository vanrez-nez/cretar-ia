# Task 04 — Hotkey input layer

## Goal
Build a strict hotkey edge ingress that emits normalized events only.

## Architecture decisions
- Keep production listener as native input listener, but isolate it from domain logic.
- Event normalization is stateless where possible, with minimal per-key timing state in adapter only.
- Keep fallback adapter available in the same abstraction so platform-specific behavior can be swapped without touching FSM.

## Files to create
- `src-tauri/src/hotkey/input/mod.rs`
- `src-tauri/src/hotkey/input/listener.rs`
- `src-tauri/src/hotkey/input/stabilizer.rs`
- `src-tauri/src/hotkey/input/traits.rs`

## Files to modify
- `src-tauri/src/hotkey.rs`
- `src-tauri/src/config.rs` (interaction config consumed as source mapping only)

## Revision details
- Define listener trait:
  - `start_listener(cfg, tx)` returns handle + runtime diagnostics.
- Normalize events to `HotkeyEdgeEvent::{Pressed, Released, Repeat, CancelPressed, Ignored}`.
- Debounce and repeat policy:
  - one-tap debounce window (configurable, default from current behavior baseline),
  - repeat edge ignored before FSM can observe.
- Modifiers and trigger-key state remain internal to stabilizer only.
- Add `HotkeyInputCoordinator` internal structure that emits typed hotkey events for orchestrator.

## Acceptance checks
- Press auto-repeat yields exactly one `Pressed` event for one physical cycle.
- Release is emitted once even if multiple release-like noise arrives.
- Input callback path remains non-blocking and returns promptly under burst.
