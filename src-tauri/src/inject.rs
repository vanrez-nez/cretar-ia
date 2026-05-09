use crate::config::{AudioCaptureConfig, OutputConfig, OutputMode};
use crate::domain::TextInjectionStep;
use crate::permissions::PermissionState;
use anyhow::{anyhow, Result};
use arboard::Clipboard;
use rdev::{simulate, EventType, Key};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static IS_DELIVERING_TEXT: AtomicBool = AtomicBool::new(false);

pub async fn deliver_text(_audio_cfg: &AudioCaptureConfig, cfg: &OutputConfig, text: &str) -> Result<()> {
    let _guard = DeliveryGuard::acquire()?;
    let plan = injection_plan(cfg.mode.clone());
    log::debug!(
        "delivery requested: mode={:?}, chars={}, paste_delay={}ms, plan={}",
        cfg.mode,
        text.len(),
        cfg.paste_delay_ms,
        describe_plan(&plan)
    );
    write_clipboard(text).await?;

    if cfg.mode == OutputMode::ClipboardPaste {
        log::debug!("pasting with configured delay");
        if cfg.paste_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(cfg.paste_delay_ms)).await;
        }
        if !paste_automation_allowed().await {
            log::warn!(
                "paste automation skipped; fallback engaged: {} (accessibility permission denied)",
                TextInjectionStep::ClipboardOnlyFallback.as_str()
            );
            return Ok(());
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

async fn write_clipboard(text: &str) -> Result<()> {
    log::debug!("delivery step: {}", TextInjectionStep::ClipboardWrite.as_str());
    let text = text.to_string();
    tokio::task::spawn_blocking(move || write_clipboard_blocking(&text))
        .await
        .map_err(|err| anyhow!("clipboard task failed: {err}"))?
}

fn write_clipboard_blocking(text: &str) -> Result<()> {
    let mut clipboard = Clipboard::new()?;
    clipboard.set_text(text.to_string())?;
    std::thread::sleep(Duration::from_millis(50));
    let actual = clipboard.get_text()?;
    if actual != text {
        return Err(anyhow!(
            "clipboard verification failed: expected {} chars, got {} chars",
            text.chars().count(),
            actual.chars().count()
        ));
    }
    log::info!("clipboard updated and verified");
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
    tokio::task::spawn_blocking(press_paste_combo_blocking)
        .await
        .map_err(|err| anyhow!("paste task failed: {err}"))?
}

fn press_paste_combo_blocking() -> Result<()> {
    #[cfg(target_os = "macos")]
    return press_paste_combo_macos();

    #[cfg(not(target_os = "macos"))]
    press_paste_combo_rdev()
}

#[cfg(target_os = "macos")]
fn press_paste_combo_macos() -> Result<()> {
    let backends: [(&str, fn() -> Result<()>); 3] = [
        ("cgevent", press_paste_combo_cgevent),
        ("applescript", press_paste_combo_applescript),
        ("rdev", press_paste_combo_rdev),
    ];

    let mut last_error = None;
    for (name, backend) in backends {
        log::debug!("paste backend attempt: {name}");
        match backend() {
            Ok(()) => {
                log::info!("paste backend succeeded: {name}");
                return Ok(());
            }
            Err(err) => {
                log::warn!("paste backend failed: {name}: {err}");
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("no paste backend executed")))
}

#[cfg(target_os = "macos")]
fn press_paste_combo_cgevent() -> Result<()> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    const KEY_V: u16 = 0x09;
    const EVENT_SETTLE_MS: u64 = 35;

    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| anyhow!("failed to create CGEventSource"))?;
    let key_down = CGEvent::new_keyboard_event(source.clone(), KEY_V, true)
        .map_err(|_| anyhow!("failed to create Cmd+V key down event"))?;
    key_down.set_flags(CGEventFlags::CGEventFlagCommand);
    key_down.post(CGEventTapLocation::HID);
    std::thread::sleep(Duration::from_millis(EVENT_SETTLE_MS));

    let key_up = CGEvent::new_keyboard_event(source, KEY_V, false)
        .map_err(|_| anyhow!("failed to create Cmd+V key up event"))?;
    key_up.set_flags(CGEventFlags::CGEventFlagCommand);
    key_up.post(CGEventTapLocation::HID);
    std::thread::sleep(Duration::from_millis(EVENT_SETTLE_MS));

    Ok(())
}

#[cfg(target_os = "macos")]
fn press_paste_combo_applescript() -> Result<()> {
    let script = r#"tell application "System Events" to keystroke "v" using {command down}"#;
    let output = std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|err| anyhow!("failed to run osascript: {err}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(anyhow!(
        "osascript exited with status {:?}: {}",
        output.status.code(),
        stderr
    ))
}

fn press_paste_combo_rdev() -> Result<()> {
    #[cfg(target_os = "macos")]
    let modifiers = [Key::MetaLeft, Key::MetaRight];

    #[cfg(not(target_os = "macos"))]
    let modifiers = [Key::ControlLeft, Key::ControlRight];

    std::thread::sleep(Duration::from_millis(50));

    for attempt in 1..=2 {
        let result = press_paste_combo_with_modifiers(&modifiers);
        match result {
            Ok(()) => return Ok(()),
            Err(err) if attempt < 2 => {
                log::warn!("paste combo attempt {attempt} failed: {err}; retrying");
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => return Err(err),
        }
    }

    Ok(())
}

fn press_paste_combo_with_modifiers(modifiers: &[Key]) -> Result<()> {
    let mut modifier = None;
    for candidate in modifiers.iter() {
        if simulate(&EventType::KeyPress(*candidate)).is_ok() {
            modifier = Some(*candidate);
            break;
        }
        log::warn!("paste modifier failed: {:?}", candidate);
    }
    let modifier = modifier.ok_or_else(|| anyhow!("no paste modifier key worked"))?;

    log::debug!("paste combo: modifier selected {:?}", modifier);
    std::thread::sleep(Duration::from_millis(50));

    log::trace!("paste combo: press V");
    if let Err(err) = simulate(&EventType::KeyPress(Key::KeyV)) {
        log::warn!("failed to press V: {err:?}");
        let _ = simulate(&EventType::KeyRelease(modifier));
        return Err(err.into());
    }
    std::thread::sleep(Duration::from_millis(50));

    log::trace!("paste combo: release V");
    if let Err(err) = simulate(&EventType::KeyRelease(Key::KeyV)) {
        log::warn!("failed to release V: {err:?}");
        let _ = simulate(&EventType::KeyRelease(modifier));
        return Err(err.into());
    }
    std::thread::sleep(Duration::from_millis(50));

    log::trace!("paste combo: release modifier");
    if let Err(err) = simulate(&EventType::KeyRelease(modifier)) {
        log::warn!("failed to release paste modifier: {err:?}");
        return Err(err.into());
    }
    std::thread::sleep(Duration::from_millis(50));

    Ok(())
}

async fn paste_automation_allowed() -> bool {
    #[cfg(target_os = "macos")]
    {
        matches!(
            crate::permissions::check_accessibility_permission().await,
            PermissionState::Granted
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = PermissionState::Granted;
        true
    }
}

struct DeliveryGuard;

impl DeliveryGuard {
    fn acquire() -> Result<Self> {
        if IS_DELIVERING_TEXT.swap(true, Ordering::SeqCst) {
            return Err(anyhow!("text delivery already in progress"));
        }
        Ok(Self)
    }
}

impl Drop for DeliveryGuard {
    fn drop(&mut self) {
        IS_DELIVERING_TEXT.store(false, Ordering::SeqCst);
    }
}
