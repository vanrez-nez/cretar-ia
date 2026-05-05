use crate::config::{AudioCaptureConfig, OutputConfig, OutputMode};
use anyhow::{anyhow, Result};
use arboard::Clipboard;
use rdev::{simulate, EventType, Key};
use std::time::Duration;

pub async fn deliver_text(_audio_cfg: &AudioCaptureConfig, cfg: &OutputConfig, text: &str) -> Result<()> {
    log::debug!(
        "delivery requested: mode={:?}, chars={}, paste_delay={}ms",
        cfg.mode,
        text.len(),
        cfg.paste_delay_ms
    );
    let mut clipboard = Clipboard::new()?;
    clipboard.set_text(text.to_string())?;
    log::info!("clipboard updated");

    if cfg.mode == OutputMode::ClipboardPaste {
        log::debug!("pasting with configured delay");
        if cfg.paste_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(cfg.paste_delay_ms)).await;
        }
        press_paste_combo().await?;
    }

    Ok(())
}

async fn press_paste_combo() -> Result<()> {
    log::debug!("injecting paste shortcut");

    #[cfg(not(target_os = "macos"))]
    let modifiers = [Key::ControlLeft, Key::ControlRight];
    #[cfg(target_os = "macos")]
    let modifiers = [Key::MetaLeft, Key::MetaRight];

    let mut modifier = None;
    for candidate in modifiers.iter() {
        if simulate(&EventType::KeyPress(*candidate)).is_ok() {
            modifier = Some(*candidate);
            break;
        }
        log::warn!("paste modifier failed: {:?}", candidate);
    }
    let modifier = match modifier {
        Some(modifier) => modifier,
        None => {
            return Err(anyhow!("no paste modifier key worked"));
        }
    };

    log::debug!("paste combo: modifier selected {:?}", modifier);
    tokio::time::sleep(Duration::from_millis(10)).await;

    log::trace!("paste combo: press V");
    if let Err(err) = simulate(&EventType::KeyPress(Key::KeyV)) {
        log::warn!("failed to press V: {err:?}");
        let _ = simulate(&EventType::KeyRelease(modifier));
        return Err(err.into());
    }
    tokio::time::sleep(Duration::from_millis(2)).await;

    log::trace!("paste combo: release V");
    if let Err(err) = simulate(&EventType::KeyRelease(Key::KeyV)) {
        log::warn!("failed to release V: {err:?}");
        let _ = simulate(&EventType::KeyRelease(modifier));
        return Err(err.into());
    }
    tokio::time::sleep(Duration::from_millis(2)).await;

    log::trace!("paste combo: release modifier");
    if let Err(err) = simulate(&EventType::KeyRelease(modifier)) {
        log::warn!("failed to release paste modifier: {err:?}");
        return Err(err.into());
    }

    Ok(())
}
