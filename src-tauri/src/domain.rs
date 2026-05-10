//! Domain-level primitives shared across runtime + platform modules.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextInjectionStep {
    ClipboardWrite,
    PasteShortcut,
    ClipboardOnlyFallback,
}

impl TextInjectionStep {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClipboardWrite => "clipboard_write",
            Self::PasteShortcut => "paste_shortcut",
            Self::ClipboardOnlyFallback => "clipboard_only_fallback",
        }
    }
}

#[allow(unused_imports)]
pub use crate::contracts::{
    HotkeyEvent, PipelineMode, PipelinePhase, PipelineState, RecordingCommand, RecordingErrorCode,
    RecordingEvent, RecoveryHint, SessionStatus,
};
