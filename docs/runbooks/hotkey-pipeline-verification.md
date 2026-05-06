# Hotkey pipeline verification runbook

Goal: validate FSM + orchestrator behavior before merging task 11 changes.

## 1) Normal push-to-talk flow

- Start the app with settings defaults and ensure tray/UI is visible.
- Press the configured shortcut once.
- Confirm `SessionStatus` indicates `starting` then `recording`.
- Release shortcut once.
- Confirm pipeline moves through `stopping` and then `processing`.
- Confirm the next status is `idle` and source includes `processing_completed`.

## 2) Toggle flow and anti-repeat

- Change interaction mode to `toggle`.
- Press shortcut once, wait for `recording`.
- Press again (without release) to stop.
- Hold shortcut and confirm no duplicate `start_requested`/`push_release_stop` events appear.
- Confirm exactly one start/stop transition for the burst.

## 3) Worker failure recovery

- Simulate start/stop/processing worker failures by forcing corresponding events:
  - `audio_start_failed` from worker path.
  - `audio_stop_failed` from worker path.
  - `process_failed` from worker path.
- Confirm transitions land in `error` with the expected `error_hint` (`retry_start`, `retry_stop`, `retry_processing`).
- Trigger `cancel` from error/recovering where required and verify recovery sequence to `recovering` then `idle`.

## 4) Mic denied / device failure

- Configure an invalid `audio.input_device` and trigger a fresh start.
- Confirm status moves to `error` and `error_code=audio_init`.
- Capture last event (`audio_start_failed`) and hint (`retry_start` / `manual` depending on config) before manual remediation.

## 5) Delayed stop path

- Start recording, hold a long time, then release.
- Confirm timeout handling and recovery hints align with current policy.
- Verify no duplicate stop outcomes are emitted for one release event.

## 6) Burst with auto-repeat and queue saturation

- Hold the hotkey and send repeated hotkey events quickly (or use hardware auto-repeat).
- Confirm `recording.queue_saturated` events appear with monotonic status `seq`.
- Verify saturation transitions produce `manual`/`retry_worker` hints according to pipeline policy.
