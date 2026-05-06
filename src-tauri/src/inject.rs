use crate::config::{AudioCaptureConfig, OutputConfig, OutputMode};
use crate::domain::TextInjectionStep;
use anyhow::{anyhow, Result};
use arboard::Clipboard;
use rdev::{simulate, EventType, Key};
use std::time::Duration;

pub async fn deliver_text(_audio_cfg: &AudioCaptureConfig, cfg: &OutputConfig, text: &str) -> Result<()> {
    let plan = injection_plan(cfg.mode.clone());
    log::debug!(
        "delivery requested: mode={:?}, chars={}, paste_delay={}ms, plan={}",
        cfg.mode,
        text.len(),
        cfg.paste_delay_ms,
        describe_plan(&plan)
    );
    write_clipboard(text)?;

    if cfg.mode == OutputMode::ClipboardPaste {
        log::debug!("pasting with configured delay");
        if cfg.paste_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(cfg.paste_delay_ms)).await;
        }
        if let Err(err) = press_paste_combo().await {
            log::warn!(
                "paste automation failed; fallback engaged: {} ({err})",
                TextInjectionStep::ClipboardOnlyFallback.as_str()
            );
        }
    }

    Ok(())
}

fn write_clipboard(text: &str) -> Result<()> {
    log::debug!("delivery step: {}", TextInjectionStep::ClipboardWrite.as_str());
    let mut clipboard = Clipboard::new()?;
    clipboard.set_text(text.to_string())?;
    log::info!("clipboard updated");
    Ok(())
}

fn injection_plan(mode: OutputMode) -> Vec<TextInjectionStep> {
    match mode {
        OutputMode::ClipboardOnly => vec![TextInjectionStep::ClipboardWrite],
        OutputMode::ClipboardPaste => vec![
            TextInjectionStep::ClipboardWrite,
            TextInjectionStep::PasteShortcut,
            TextInjectionStep::ClipboardOnlyFallback,
        ],
    }
}

fn describe_plan(plan: &[TextInjectionStep]) -> String {
    plan.iter()
        .map(|step| step.as_str())
        .collect::<Vec<_>>()
        .join(">")
}

async fn press_paste_combo() -> Result<()> {
    log::debug!("delivery step: {}", TextInjectionStep::PasteShortcut.as_str());

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
