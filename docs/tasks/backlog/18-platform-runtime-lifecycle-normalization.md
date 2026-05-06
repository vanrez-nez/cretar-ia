# Task 18 — Normalize platform runtime lifecycle behavior

## Goal
Reduce divergence between platform-specific runtime startup/shutdown flows while preserving existing OS-specific entrypoints.

## Architecture decisions
- Keep macOS and non-macOS entrypoints behind a shared runtime adapter contract.
- Keep tray behavior where needed, but centralize orchestration lifecycle ownership (start, stop, wait).
- No new feature flags or runtime behavior changes unless required to align structure.

## Files to create
- None.

## Files to modify
- `src-tauri/src/runtime/engine.rs`
- `src-tauri/src/runtime/compat.rs`
- `src-tauri/src/main.rs`

## Revision details
1. Extract shared lifecycle scaffolding from both paths.
2. Align shutdown flow and thread/task boundaries to the same conceptual phases.
3. Keep OS-specific behavior documented at the edge only.
4. Ensure event and telemetry handoff follows same order independent of platform.

## Acceptance checks
- Equivalent startup and teardown steps observable across both platform paths.
- No change in user-facing behavior for current supported commands.
- Reduced conditional branching in core orchestration path.
