pub mod audio_level;
pub mod commands;
pub mod errors;
pub mod events;
pub mod status;

pub use audio_level::AudioLevelSample;
pub use commands::RecordingCommand;
pub use errors::{RecordingErrorCode, RecoveryHint};
pub use events::{
    new_envelope, next_seq, EventEnvelope, HotkeyEvent, PipelineMode, PipelinePhase, PipelineState,
    RecordingEvent, CURRENT_SCHEMA_VERSION,
};
pub use status::SessionStatus;
