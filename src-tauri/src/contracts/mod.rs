pub mod commands;
pub mod errors;
pub mod events;
pub mod status;

pub use commands::RecordingCommand;
pub use errors::{RecordingErrorCode, RecoveryHint};
pub use events::{
    next_seq,
    new_envelope,
    EventEnvelope,
    HotkeyEvent,
    PipelineMode,
    PipelinePhase,
    PipelineState,
    RecordingEvent,
    CURRENT_SCHEMA_VERSION,
};
pub use status::SessionStatus;
