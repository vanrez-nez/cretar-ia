use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingErrorCode {
    HotkeyParse,
    AudioInit,
    AudioStop,
    WorkerTimeout,
    QueueOverflow,
    Processing,
    ConfigInvalid,
    Unknown,
}

impl RecordingErrorCode {
    pub const fn is_recoverable(self) -> bool {
        match self {
            Self::HotkeyParse => false,
            Self::AudioInit => true,
            Self::AudioStop => true,
            Self::WorkerTimeout => true,
            Self::QueueOverflow => true,
            Self::Processing => true,
            Self::ConfigInvalid => false,
            Self::Unknown => true,
        }
    }
}

impl fmt::Display for RecordingErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::HotkeyParse => "hotkey_parse",
            Self::AudioInit => "audio_init",
            Self::AudioStop => "audio_stop",
            Self::WorkerTimeout => "worker_timeout",
            Self::QueueOverflow => "queue_overflow",
            Self::Processing => "processing",
            Self::ConfigInvalid => "config_invalid",
            Self::Unknown => "unknown",
        };
        write!(f, "{text}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryHint {
    NoRecovery,
    RetryProcessing,
    RetryStart,
    RetryStop,
    RetryWorker,
    Manual,
}

impl fmt::Display for RecoveryHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::NoRecovery => "no_recovery",
            Self::RetryProcessing => "retry_processing",
            Self::RetryStart => "retry_start",
            Self::RetryStop => "retry_stop",
            Self::RetryWorker => "retry_worker",
            Self::Manual => "manual",
        };
        write!(f, "{text}")
    }
}

