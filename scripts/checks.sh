#!/usr/bin/env bash
set -euo pipefail

echo "[0/4] backend: rustfmt"
cargo fmt --all -- --check

echo "[1/4] backend: build"
cargo build -p cretar-ia

echo "[2/4] backend: lint"
cargo clippy -p cretar-ia --all-targets -- -D warnings

echo "[3/4] frontend: typecheck"
npm run check

echo "[4/4] frontend: build"
npm run build

echo "checks complete"
