# Task 17 — Injection fallback strategy and output mode normalization

## Goal
Make text output insertion resilient by introducing a small fallback strategy when primary paste automation fails, without changing the command contract.

## Architecture decisions
- Keep existing injector entrypoint and signature.
- Add configurable fallback strategy sequence internally (primary automation first, fallback alternatives after failure).
- Avoid introducing a new UI flow; keep failure reporting in existing error/status channels.

## Files to create
- None.

## Files to modify
- `src-tauri/src/inject.rs`
- `src-tauri/src/domain.rs`
- `src-tauri/src/recording/fsm.rs`

## Revision details
1. Add explicit fallback decision points for injection failures.
2. Add deterministic order: retry policy for non-destructive fallback only.
3. Preserve existing success path and status signaling.
4. Document when fallback mode is engaged and why.

## Acceptance checks
- Injection failure no longer hard-fails the whole processing flow when a fallback can continue.
- Fallback strategy is deterministic and auditable.
- No new dependencies are required for alternate output method.
