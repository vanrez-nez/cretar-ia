#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod noop;

#[cfg(target_os = "macos")]
pub use macos::MediaPauseController;
#[cfg(not(target_os = "macos"))]
pub use noop::MediaPauseController;
