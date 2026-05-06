# Task 16 — Settings command path normalization via shared service

## Goal
Reduce duplicate config load/validate/save logic in IPC command handlers while keeping command surface unchanged.

## Architecture decisions
- Introduce a shared settings/settings-service abstraction only within command/config area.
- Keep public command signatures stable (`get_settings`, `update_settings`) unless compile constraints require minimal typed DTOs.
- Keep validation centralized and deterministic.

## Files to create
- `src-tauri/src/commands/settings_service.rs` (or `src-tauri/src/commands/mod.rs` helper section if preferred)

## Files to modify
- `src-tauri/src/commands/settings.rs`
- `src-tauri/src/config.rs`

## Revision details
1. Extract a single internal flow for load + validate + save.
2. Reduce duplicated I/O and parsing across existing setting commands.
3. Centralize error shaping to avoid subtle drift in returned command errors.
4. Add minimal tests for config path, validation, and update semantics if existing command tests can be extended without refactor cost.

## Acceptance checks
- One canonical settings update path is used by all settings commands.
- Functionality of get/update remains stable from caller perspective.
- Error/validation messages remain explicit and less duplicated.
