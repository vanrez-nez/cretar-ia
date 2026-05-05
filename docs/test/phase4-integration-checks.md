# Phase 4 integration checks (hardening)

## 1) Startup check

- Start the app binary with the settings feature to confirm config bootstrap still works:
  - `cd src-tauri && cargo run --features tray -- --settings`
- Confirm:
  - `.cretar-ia/config.json` is created on first run.
  - no raw panic or status string formatting errors appear in logs.
  - `settings` IPC now uses command module and returns valid JSON.

## 2) Runtime status-model check

- Verify transition values are driven from `RuntimeSessionState` / `AppRuntimeStatus` instead of raw status strings:
  - Confirm `AppRuntimeStatus` enum variants in `src-tauri/src/domain/event.rs` include: `Idle`, `Recording`, `Sending`, `Success`, `Error`, `Shutdown`.
  - Search in `src-tauri/src/runtime/engine.rs` for `session_state.status`.
  - Search in `src-tauri/src/tray.rs` for `icon_state`.
- Confirm these terminal states exist:
  - `Idle`
  - `Recording`
  - `Sending`
  - `Success`
  - `Error`
  - `Shutdown`
- Confirm tray status updates in runtime loops always read `session_state.status` and do not hand-roll status values.

## 3) Frontend state model check

- Verify settings screen state is read from one typed source:
  - `appState` is part of `useSettingsStore` state.
  - `src/App.tsx` derives user-facing status from `appState.status`.
- Validate transitions:
  - `"loading"` when fetch starts.
  - `"sending"` while save is in-flight.
  - `"success"` after save/load success.
  - `"error"` on load/save failure.

## 4) Manual recording flow check

- Run the app in normal mode (no settings UI) and exercise shortcut flow:
  - Start recording (`Start` action).
  - Stop recording (`Stop` action).
  - Confirm no panic and that status transitions follow:
    - Idle -> Recording -> Sending -> Idle.
- Confirm tray status icon follows typed state (recording/sending/error/idle equivalent).
