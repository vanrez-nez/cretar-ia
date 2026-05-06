use crate::contracts::errors::{RecordingErrorCode, RecoveryHint};
use crate::contracts::status::SessionStatus;
use crate::recording::state::RecordingState;
use std::time::Instant;

pub fn status(
    state: &RecordingState,
    seq: u64,
    phase_started_at: Instant,
    error_code: Option<RecordingErrorCode>,
    error_hint: RecoveryHint,
    last_event: impl Into<String>,
) -> SessionStatus {
    let uptime_ms = phase_started_at.elapsed().as_millis().min(u64::MAX as u128) as u64;

    SessionStatus {
        state: state.phase,
        mode: state.mode,
        error_code,
        error_hint,
        session_id: state.session_id,
        seq,
        phase_elapsed_ms: uptime_ms,
        source: last_event.into(),
    }
}
