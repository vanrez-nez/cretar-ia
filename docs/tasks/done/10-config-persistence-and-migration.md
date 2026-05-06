# Task 10 — Config persistence and migration

## Goal
Introduce pipeline configuration extensions while keeping existing config behavior compatible.

## Architecture decisions
- Add optional pipeline tuning block; legacy files remain loadable.
- Use explicit defaults for all new fields to avoid null/absent ambiguity.
- Validation happens before orchestrator start; invalid config is rejected deterministically.

## Files to create
- `src-tauri/src/config/migration.rs` (if migration helpers required)

## Files to modify
- `src-tauri/src/config.rs`
- `src-tauri/src/commands/settings.rs`
- `src-tauri/src/runtime/engine.rs` (consume validated config state)
- `README.md` (document new config fields)

## Revision details
- Add optional fields:
  - debounce duration,
  - settle timeout,
  - queue capacities,
  - queue saturation policy,
  - recovery strategy flags,
  - optional max recording duration enforcement.
- Add validation:
  - numeric bounds (e.g., non-negative, sane upper limits),
  - shortcut parse validity,
  - contradictory mode combinations fail fast.
- Migration details:
  - `interaction` remains root-compatible,
  - missing pipeline fields use defaults,
  - unknown fields preserved where serde behavior allows.

## Acceptance checks
- Existing installs load and start without manual migration.
- Invalid config throws clear typed messages before runtime loop.
- Persisting config round-trips without field loss.
