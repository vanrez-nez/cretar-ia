use crate::contracts::errors::{RecordingErrorCode, RecoveryHint};
use crate::contracts::events::{PipelineMode, PipelinePhase};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatus {
    pub state: PipelinePhase,
    pub mode: PipelineMode,
    pub error_code: Option<RecordingErrorCode>,
    pub error_hint: RecoveryHint,
    pub session_id: u64,
    pub seq: u64,
    pub phase_elapsed_ms: u64,
    pub source: String,
}

impl SessionStatus {
    pub const fn with_defaults(
        state: PipelinePhase,
        mode: PipelineMode,
        session_id: u64,
        seq: u64,
        source: String,
    ) -> Self {
        Self {
            state,
            mode,
            error_code: None,
            error_hint: RecoveryHint::NoRecovery,
            session_id,
            seq,
            phase_elapsed_ms: 0,
            source,
        }
    }
}

impl fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "session_status(session={},seq={},state={},mode={},error_code={:?},error_hint={},phase_elapsed_ms={},source={})",
            self.session_id,
            self.seq,
            self.state,
            self.mode,
            self.error_code,
            self.error_hint,
            self.phase_elapsed_ms,
            self.source
        )
    }
}

