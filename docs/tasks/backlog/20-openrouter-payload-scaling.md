# Task 20 — OpenRouter payload scaling and efficiency normalization

## Goal
Improve handling of large audio payloads by simplifying request size and memory behavior in OpenRouter integration without altering endpoint contracts.

## Architecture decisions
- Keep OpenRouter request format where possible.
- Introduce size checks and threshold behavior before base64 serialization.
- Avoid full in-memory duplication where avoidable.

## Files to create
- None.

## Files to modify
- `src-tauri/src/openrouter.rs`
- `src-tauri/src/config.rs`

## Revision details
1. Add pre-flight validation for payload size and clear user-facing error on limit breach.
2. Reduce redundant allocation during serialization if practical.
3. Document fallback behavior for oversized recordings (truncate/error/retry strategy).
4. Keep API call surface unchanged unless required by simplification.

## Acceptance checks
- Oversized audio input does not silently attempt a request path that can OOM.
- Error path is explicit and actionable.
- Existing success path remains stable for normal-size payloads.
