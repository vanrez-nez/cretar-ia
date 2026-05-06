mod engine;
pub(crate) mod compat;

#[cfg(not(target_os = "macos"))]
pub use engine::run_non_macos;
#[cfg(target_os = "macos")]
pub use engine::run_macos;
