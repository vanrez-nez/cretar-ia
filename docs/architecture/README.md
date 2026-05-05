# Architecture and Refactor Plan (by phase)

This repository is being refactored in three phases.

## Practice extraction source

- [VoxNote reference extraction notes](voxnote-practices-extract.md)

## Phase 1: File structure and organization

Goals:

- Keep runtime platform code (tray/hotkey/audio/inject/openrouter) in stable Rust entry modules.
- Push all settings/runtime IPC handlers behind command modules.
- Keep configuration schema mirrored in frontend types.

Reference practices extracted from `external/voxnote`:

- Tauri crate keeps commands in a dedicated `commands/` namespace and groups handlers by domain.
- Tauri crate entrypoint is thin: plugin/state setup + generated handler registration in `lib.rs`.
- Shared docs are colocated with implementation (`docs/architecture`, `docs/test`).

Current implementation:

- Added `src-tauri/src/domain` for cross-module events (`AppEvent`, `AppPhase`, `WorkEvent`).
- Added `src-tauri/src/commands/settings.rs` + `src-tauri/src/commands/mod.rs`.
- Added `src-tauri/src/runtime` split (`mod.rs`, `engine.rs`) for non-UI orchestration.
- Kept platform adapters in `src-tauri/src/{tray,hotkey,audio,audio_cues,inject}`.

## Phase 2: Rust modules

Goals:

- Separate pure orchestration from platform-specific behavior.
- Use explicit states/events for recorder lifecycle.
- Keep settings load/save responsibilities in one command module.

Reference practices extracted from `external/voxnote`:

- Shared application state should be centralized in an app state object managed by Tauri (`manage`), so commands and runtime can share ownership.
- Command surfaces should be narrow and explicit (`get_settings`, `update_settings`, etc.).
- Runtime/core logic should stay independent from shell/platform glue whenever possible.

Current implementation:

- `src-tauri/src/runtime/{mod,engine}.rs` contains:
  - startup variants (`run_non_macos`, `run_macos`)
  - event loop (`run_core`, `run_core_macos`)
  - transcription worker (`process_recording_work`)
- `AppEvent` flows from input adapters (`hotkey`) into runtime event loop.
- Settings commands moved to `commands/settings.rs`.

## Phase 3: Frontend

Goals:

- Make settings screen state-driven via an external store.
- Use typed IPC wrappers, not raw string-only invoke.
- Keep JSON shape stable by sharing interfaces with backend config.

Reference practices extracted from `external/voxnote`:

- Frontend uses dedicated typed stores per domain (`settings`, `recording`, `note`, etc.) rather than local component-only state.
- IPC wrapper is centralized (`tauriInvoke`) and reused.
- Tauri event hooks (`listen`) are isolated in small hooks.

Current implementation:

- Added `src/lib/types.ts` (`AppConfig` + defaults).
- Added `src/hooks/useTauriIPC.ts` typed `tauriInvoke` and `useTauriEvent`.
- Added `src/stores/settingsStore.ts` with clear loading/saving/error state.
- Updated `src/App.tsx` to consume store selectors and show phase-aware status.

## Phase 4 (optional): hardening pass

- Add shared app state type that captures runtime/session/runtime-config status instead of raw strings.
- Add small integration checks for startup/settings/recording transitions.
- Move domain-specific constants to dedicated config modules (instead of inline strings).

## Phase 4 (implemented)

- Added shared backend runtime session model:
  - `RuntimeSessionState` and `AppRuntimeStatus` in [`src-tauri/src/domain/event.rs`](src-tauri/src/domain/event.rs).
- Replaced raw tray status string calls with typed status updates in runtime:
  - [`src-tauri/src/runtime/engine.rs`](src-tauri/src/runtime/engine.rs)
  - [`src-tauri/src/tray.rs`](src-tauri/src/tray.rs)
- Added typed frontend app state:
  - [`src/lib/runtime.ts`](src/lib/runtime.ts)
  - `appState` in [`src/stores/settingsStore.ts`](src/stores/settingsStore.ts)
- Added phase 4 integration checklist:
  - [`docs/test/phase4-integration-checks.md`](docs/test/phase4-integration-checks.md)
