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
use std::path::Path;

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

pub(crate) fn init_logging(base_dir: &Path) -> Result<()> {
    let log_dir = base_dir.join("logs");
    let _ = std::fs::create_dir_all(&log_dir);

    let logger = || {
        fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{}][{}][{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                record.level(),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::Debug)
        .chain(std::io::stdout())
    };

    match fern::log_file(log_dir.join("app.log")) {
        Ok(file) => logger().chain(file).apply()?,
        Err(err) => {
            eprintln!("failed to initialize file logger: {err}");
            logger().apply()?;
        }
    }

    Ok(())
}
