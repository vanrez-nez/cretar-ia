use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingCommand {
    StartRecording,
    StopRecording,
    CancelRecording,
    ForceStop,
    RunProcessing,
    CancelProcessing,
    Shutdown,
}

impl fmt::Display for RecordingCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::StartRecording => "start_recording",
            Self::StopRecording => "stop_recording",
            Self::CancelRecording => "cancel_recording",
            Self::ForceStop => "force_stop",
            Self::RunProcessing => "run_processing",
            Self::CancelProcessing => "cancel_processing",
            Self::Shutdown => "shutdown",
        };
        write!(f, "{text}")
    }
}
