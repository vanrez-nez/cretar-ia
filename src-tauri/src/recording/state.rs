use crate::config::InteractionMode;
use crate::contracts::errors::RecoveryHint;
use crate::contracts::events::{PipelineMode, PipelinePhase};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RecordingState {
    pub phase: PipelinePhase,
    pub mode: PipelineMode,
    pub session_id: u64,
    pub seq: u64,
    pub recovery_hint: RecoveryHint,
    pub last_reason: Option<String>,
}

impl Default for RecordingState {
    fn default() -> Self {
        Self {
            phase: PipelinePhase::Idle,
            mode: PipelineMode::PushToTalk,
            session_id: 0,
            seq: 0,
            recovery_hint: RecoveryHint::NoRecovery,
            last_reason: None,
        }
    }
}

impl RecordingState {
    pub const fn new(mode: PipelineMode, session_id: u64) -> Self {
        Self {
            phase: PipelinePhase::Idle,
            mode,
            session_id,
            seq: 0,
            recovery_hint: RecoveryHint::NoRecovery,
            last_reason: None,
        }
    }

    pub fn with_mode(mut self, mode: PipelineMode) -> Self {
        self.mode = mode;
        self
    }

    pub const fn with_phase(mut self, phase: PipelinePhase) -> Self {
        self.phase = phase;
        self
    }

    pub fn with_recovery_hint(mut self, hint: RecoveryHint) -> Self {
        self.recovery_hint = hint;
        self
    }

    pub fn with_reason(mut self, reason: Option<String>) -> Self {
        self.last_reason = reason;
        self
    }

    pub fn with_error(mut self, reason: impl Into<String>, hint: RecoveryHint) -> Self {
        self.phase = PipelinePhase::Error;
        self.last_reason = Some(reason.into());
        self.recovery_hint = hint;
        self
    }

    pub fn with_recovery(mut self, reason: impl Into<String>, hint: RecoveryHint) -> Self {
        self.phase = PipelinePhase::Recovering;
        self.last_reason = Some(reason.into());
        self.recovery_hint = hint;
        self
    }

    pub fn next_seq(mut self) -> Self {
        self.seq = self.seq.saturating_add(1);
        self
    }

    pub fn next_session(mut self) -> Self {
        self.session_id = self.session_id.saturating_add(1);
        if self.session_id == 0 {
            self.session_id = 1;
        }
        self
    }
}

impl From<InteractionMode> for PipelineMode {
    fn from(value: InteractionMode) -> Self {
        match value {
            InteractionMode::PushToTalk => PipelineMode::PushToTalk,
            InteractionMode::Toggle => PipelineMode::Toggle,
        }
    }
}
