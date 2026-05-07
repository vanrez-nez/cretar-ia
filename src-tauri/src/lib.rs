mod audio;
mod audio_cues;
mod config;
mod domain;
pub mod contracts;
mod hotkey;
mod i18n;
mod inject;
mod openrouter;
#[cfg(feature = "settings-ui")]
mod permissions;
#[cfg(feature = "settings-ui")]
mod app_host;
#[cfg(feature = "settings-ui")]
mod settings_db;
mod runtime;
mod recording;
mod tray;
#[cfg(feature = "settings-ui")]
mod commands;

use anyhow::Result;

pub fn run() -> Result<()> {
    #[cfg(feature = "settings-ui")]
    {
        return app_host::run();
    }

    #[cfg(not(feature = "settings-ui"))]
    {
        Ok(())
    }
}
