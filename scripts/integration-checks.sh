#!/usr/bin/env bash

set -euo pipefail

echo "[1/3] backend: cargo check"
cargo check -p cretar-ia

echo "[2/3] backend/domain: ensure tray status calls use AppRuntimeStatus"
if rg -n "set_status\\([^,]+,[[:space:]]*\"(recording|sending|error|done|shutdown|idle)\"" src-tauri/src/runtime src-tauri/src/tray > /dev/null; then
  echo "Legacy raw status strings still present in status call sites."
  exit 1
else
  echo "No legacy raw tray status strings found."
fi

echo "[3/3] frontend: validate shared runtime app state types"
rg -n "RuntimeSessionState|AppRuntimeStatus|appState" src-tauri/src src/stores src/lib src/App.tsx >/dev/null

echo "integration checks complete"
