# Frontend + Rust refactor test plan

## Scope

- Frontend settings flow (load/edit/save config).
- Command boundary refactor (settings + runtime separation).
- Runtime/app-event boundary stability (`hotkey` -> `runtime` state -> `tray` status).

## Priority matrix

- P0: No regression in config path, settings editor launch, start/stop hotkey flow, transcription dispatch.
- P1: Store-driven UI updates and typed IPC usage.
- P2: Documentation consistency.
- P3: Traceability: each feature maps to acceptance criteria and manual verification steps.

## Suggested checks

- Build:
  - `cargo build -p cretar-ia` (desktop binary crate in this repo).
  - `cargo clippy -p cretar-ia --all-targets -- -D warnings` (lint as quality gate).
  - `npm run build`.
- Sanity:
  - Start app, open settings (`--settings` feature path), load config, make a JSON edit, save, reopen, confirm persistence.
  - Validate global shortcut starts/stops recording and transcribes with configured provider.
- Quality:
  - `npx tsc --noEmit`.
  - `cargo fmt --all -- --check`.

## Phase verification map

- Phase 1: File structure
  - Confirm folder and module boundaries match README.
  - Confirm no platform-adapter imports appear in command modules.
- Phase 2: Rust modules
  - Confirm settings handlers are only in `src-tauri/src/commands`.
  - Confirm runtime lifecycle (`Start/Stop/Toggle/Quit`) still transitions `Idle -> Recording -> Sending -> Idle`.
  - Confirm tray status transitions remain functional under `--features tray`.
- Phase 3: Frontend
  - Confirm `settingsStore` is the sole source of truth for `config`, `isLoading`, `isSaving`, and `error`.
  - Confirm save/load paths use `tauriInvoke("load_config")` and `tauriInvoke("save_config")`.
  - Confirm manual/JSON-mode path does not crash when running outside Tauri.
- Phase 4: Hardening
  - Confirm `RuntimeSessionState`/`AppRuntimeStatus` is used for status transitions.
  - Confirm frontend `appState` is the central typed state for runtime/config status.
  - Run `./scripts/integration-checks.sh` and confirm all checks pass.
- Additional hardening
  - Add traceability matrix entry linking feature changes to one test case ID each (FR/PR).

