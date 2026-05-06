# Task 19 — Hard-remove legacy compatibility drift from core boundaries

## Goal
Contain legacy compatibility paths so they cannot affect new runtime behavior, while preserving migration safety.

## Architecture decisions
- Keep compatibility shims in a dedicated boundary module only.
- Prevent legacy event/domain variants from leaking into core exports unless required by migration logic.
- No functional rollback to old paths.

## Files to create
- None.

## Files to modify
- `src-tauri/src/domain/event.rs`
- `src-tauri/src/runtime/compat.rs`
- `src-tauri/src/domain.rs`

## Revision details
1. Identify and isolate compatibility-only constructs and conversions.
2. Remove legacy variants from new-state hot paths.
3. Add deprecation markers/comments where temporary compatibility remains.
4. Keep migration compatibility intact and tested.

## Acceptance checks
- Core command and runtime code paths no longer import legacy event variants directly.
- Compatibility layer remains explicit and narrow in scope.
- Legacy conversion is used only where migration requires it.
