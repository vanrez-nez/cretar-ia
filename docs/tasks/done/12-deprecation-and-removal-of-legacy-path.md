# Task 12 — Deprecate/remove legacy hotkey + recording path

## Goal
Ensure primary runtime is orchestrator-driven and legacy direct state/event paths are phased out.

## Architecture decisions
- Compatibility adapters are allowed only for external surfaces; runtime behavior is single-source in orchestrator.
- Legacy event types remain for settings UI compatibility but are not authoritative for behavior.
- Removal happens after compatibility layer proves no direct action path to recorder.

## Files to create
- `src-tauri/src/runtime/compat.rs`

## Files to modify
- `src-tauri/src/domain/event.rs` (deprecate old phase-centric API)
- `src-tauri/src/runtime/engine.rs` (replace old loop with orchestrator loop)
- `src-tauri/src/hotkey.rs` (legacy path emits contract events only)
- `src-tauri/src/lib.rs` (start path unchanged, but module route through orchestrator)

## Revision details
- Inventory direct recording actions and map each to orchestrator commands.
- Replace branch-based `AppEvent` handling in runtime with contract event intake and orchestrator status output.
- Mark legacy call sites with deprecation notes where still required for backward compatibility.
- Remove old status transitions in runtime loop and rely on snapshot stream from Task 09.

## Acceptance checks
- There is exactly one state owner: orchestrator FSM.
- Legacy runtime paths cannot directly call `audio::Recorder::start/stop`.
- Feature flags or docs show deprecation with migration guidance.
