use crate::contracts::errors::RecordingErrorCode;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const CURRENT_SCHEMA_VERSION: u16 = 1;

static GLOBAL_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineMode {
    PushToTalk,
    Toggle,
}

impl PipelineMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PushToTalk => "push_to_talk",
            Self::Toggle => "toggle",
        }
    }
}

impl fmt::Display for PipelineMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelinePhase {
    Idle,
    Starting,
    Recording,
    Stopping,
    Processing,
    Recovering,
    Error,
}

impl PipelinePhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Starting => "starting",
            Self::Recording => "recording",
            Self::Stopping => "stopping",
            Self::Processing => "processing",
            Self::Recovering => "recovering",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for PipelinePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyEvent {
    Pressed,
    Released,
    TogglePressed,
    Repeat,
    CancelPressed,
    ModeUpdate(PipelineMode),
    ShutdownRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingEvent {
    AudioStarted,
    AudioStartFailed {
        code: RecordingErrorCode,
        reason: String,
    },
    AudioStopped {
        artifact: RecordingArtifact,
    },
    AudioStopFailed {
        code: RecordingErrorCode,
        reason: String,
    },
    AudioDeviceUnavailable {
        code: RecordingErrorCode,
        reason: String,
    },
    ProcessStarted,
    ProcessCompleted,
    ProcessFailed {
        code: RecordingErrorCode,
        reason: String,
    },
    TransformFailed {
        reason: String,
    },
    QueueSaturated {
        source: String,
        dropped: u32,
    },
    TimeoutExpired,
    RecoveryCompleted,
    RecoveryFailed {
        code: RecordingErrorCode,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingArtifact {
    pub path: PathBuf,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineState {
    pub phase: PipelinePhase,
    pub mode: PipelineMode,
    pub session_id: u64,
    pub seq: u64,
    pub last_reason: Option<String>,
}

impl Default for PipelineState {
    fn default() -> Self {
        Self {
            phase: PipelinePhase::Idle,
            mode: PipelineMode::PushToTalk,
            session_id: 0,
            seq: 0,
            last_reason: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope<T> {
    pub session_id: u64,
    pub seq: u64,
    pub created_at_ms: u64,
    pub source: String,
    pub schema_version: u16,
    pub payload: T,
}

pub fn next_seq() -> u64 {
    GLOBAL_SEQ.fetch_add(1, Ordering::Relaxed)
}

pub fn new_envelope<T>(
    session_id: u64,
    source: impl Into<String>,
    payload: T,
) -> EventEnvelope<T> {
    EventEnvelope {
        session_id,
        seq: next_seq(),
        created_at_ms: current_time_ms(),
        source: source.into(),
        schema_version: CURRENT_SCHEMA_VERSION,
        payload,
    }
}

fn current_time_ms() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as u64,
        Err(_) => 0,
    }
}

impl fmt::Display for HotkeyEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Pressed => "hotkey.pressed",
            Self::Released => "hotkey.released",
            Self::TogglePressed => "hotkey.toggle_pressed",
            Self::Repeat => "hotkey.repeat",
            Self::CancelPressed => "hotkey.cancel_pressed",
            Self::ModeUpdate(mode) => {
                return write!(f, "hotkey.mode_update:{}", mode);
            }
            Self::ShutdownRequested => "hotkey.shutdown_requested",
        };
        write!(f, "{text}")
    }
}

impl fmt::Display for RecordingEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::AudioStarted => "recording.audio_started",
            Self::AudioStartFailed { code, reason } => {
                return write!(f, "recording.audio_start_failed:{code}:{reason}");
            }
            Self::AudioStopped { .. } => "recording.audio_stopped",
            Self::AudioStopFailed { code, reason } => {
                return write!(f, "recording.audio_stop_failed:{code}:{reason}");
            }
            Self::AudioDeviceUnavailable { code, reason } => {
                return write!(f, "recording.audio_device_unavailable:{code}:{reason}");
            }
            Self::ProcessStarted => "recording.process_started",
            Self::ProcessCompleted => "recording.process_completed",
            Self::ProcessFailed { code, reason } => {
                return write!(f, "recording.process_failed:{code}:{reason}");
            }
            Self::TransformFailed { reason } => {
                return write!(f, "recording.transform_failed:{reason}");
            }
            Self::QueueSaturated { source, dropped } => {
                return write!(f, "recording.queue_saturated:{source}:{dropped}");
            }
            Self::TimeoutExpired => "recording.timeout_expired",
            Self::RecoveryCompleted => "recording.recovery_completed",
            Self::RecoveryFailed { code, reason } => {
                return write!(f, "recording.recovery_failed:{code}:{reason}");
            }
        };
        write!(f, "{text}")
    }
}

impl<T: fmt::Debug> fmt::Display for EventEnvelope<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "event_envelope(session={},seq={},source={},schema_version={},payload={:?})",
            self.session_id, self.seq, self.source, self.schema_version, self.payload
        )
    }
}

impl fmt::Display for PipelineState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "pipeline_state(session={},seq={},phase={},mode={},last_reason={})",
            self.session_id,
            self.seq,
            self.phase,
            self.mode,
            self.last_reason
                .clone()
                .unwrap_or_else(|| "none".to_string())
        )
    }
}
