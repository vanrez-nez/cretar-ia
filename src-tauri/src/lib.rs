mod audio;
mod audio_cues;
mod config;
mod domain;
pub mod contracts;
mod hotkey;
mod inject;
mod openrouter;
#[cfg(feature = "settings-ui")]
mod permissions;
#[cfg(feature = "settings-ui")]
mod app_host;
mod runtime;
mod recording;
mod tray;
#[cfg(feature = "settings-ui")]
mod commands;

use anyhow::Result;
use config::AppConfig;

pub fn run() -> Result<()> {
    let cfg = AppConfig::load_or_create()?;
    cfg.validate().map_err(|err| anyhow::anyhow!("{err}"))?;
    init_logging(&cfg)?;

    log::info!(
        "starting app v{} with interaction {:?} and shortcut {}",
        env!("CARGO_PKG_VERSION"),
        cfg.interaction.mode,
        cfg.interaction.shortcut
    );
    log::info!("config path: {}", AppConfig::config_path().display());

    #[cfg(feature = "settings-ui")]
    {
        return app_host::run(cfg);
    }

    #[cfg(not(feature = "settings-ui"))]
    {
        let _ = cfg;
        Ok(())
    }
}

fn init_logging(cfg: &AppConfig) -> Result<()> {
    let _ = std::fs::create_dir_all(cfg.base_dir_path().join("logs"));

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

    match fern::log_file(config::AppConfig::log_file_path()) {
        Ok(file) => logger().chain(file).apply()?,
        Err(err) => {
            eprintln!("failed to initialize file logger: {err}");
            logger().apply()?;
        }
    }

    Ok(())
}
