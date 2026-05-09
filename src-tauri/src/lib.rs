mod audio;
mod audio_cues;
mod config;
mod domain;
pub mod contracts;
mod hotkey;
mod i18n;
mod inject;
mod media_control;
#[cfg(feature = "settings-ui")]
mod model_health;
#[cfg(feature = "settings-ui")]
mod providers;
#[cfg(feature = "settings-ui")]
mod prompts;
#[cfg(feature = "settings-ui")]
mod permissions;
#[cfg(feature = "settings-ui")]
mod app_host;
#[cfg(feature = "settings-ui")]
mod settings_db;
#[cfg(feature = "settings-ui")]
mod settings_schema;
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
