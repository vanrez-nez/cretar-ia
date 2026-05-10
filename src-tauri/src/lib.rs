#[cfg(feature = "settings-ui")]
mod app_host;
mod audio;
mod audio_cues;
#[cfg(feature = "settings-ui")]
mod commands;
mod config;
pub mod contracts;
mod domain;
mod history;
mod hotkey;
mod i18n;
mod inject;
mod media_control;
#[cfg(feature = "settings-ui")]
mod model_health;
#[cfg(feature = "settings-ui")]
mod permissions;
#[cfg(feature = "settings-ui")]
mod prompts;
#[cfg(feature = "settings-ui")]
mod providers;
mod recording;
mod runtime;
#[cfg(feature = "settings-ui")]
mod settings_db;
#[cfg(feature = "settings-ui")]
mod settings_schema;
#[cfg(feature = "settings-ui")]
mod status_widget;
mod tray;

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
