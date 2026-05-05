//! Domain-level primitives shared across runtime + platform modules.

pub mod event;

#[allow(unused_imports)]
pub use event::{AppEvent, AppPhase, AppRuntimeStatus, RuntimeSessionState, WorkEvent};
