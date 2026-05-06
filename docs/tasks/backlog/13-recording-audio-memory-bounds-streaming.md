# Task 13 — Recording audio memory bounds and streaming simplification

## Goal
Eliminate unbounded in-memory audio accumulation in the recording path and simplify the capture flow to bounded, lifecycle-safe buffering without changing user-visible behavior.

## Architecture decisions
- Keep existing recorder/workflow ownership model (same worker/thread boundaries).
- Replace unbounded sample growth with bounded buffering and explicit persistence checkpoints.
- Keep changes local to current modules to avoid introducing new cross-cutting abstractions.

## Files to create
- None.

## Files to modify
- `src-tauri/src/audio.rs`
- `src-tauri/src/recording/workers/audio_worker.rs`
- `src-tauri/src/recording/orchestrator.rs`

## Revision details
1. Introduce bounded capture handling in the recorder path (`audio.rs`) with a configurable/max-safe in-memory threshold.
2. Persist recording chunks incrementally to avoid retaining full sessions in RAM.
3. Keep current API signatures stable where possible; only add minimal internal fields/helpers.
4. Ensure stop logic flushes in-memory remainder and does not require holding all samples before file generation.

## Acceptance checks
- Long sessions do not grow memory linearly with sample count.
- No functional change in start/stop behavior from the orchestrator perspective.
- Existing recovery path continues to emit valid final audio output when stop succeeds.
- New bounded path has explicit behavior when limits are exceeded (documented and deterministic).
