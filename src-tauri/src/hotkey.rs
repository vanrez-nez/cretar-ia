use crate::config::{InteractionConfig, InteractionMode};
use crate::domain::AppEvent;
use anyhow::{anyhow, Result};
use rdev::{listen, Event, EventType, Key};
use std::sync::{Mutex};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Copy, Debug)]
struct ParsedShortcut {
    key: Key,
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
}

pub fn spawn_listener(
    cfg: InteractionConfig,
    tx: UnboundedSender<AppEvent>,
) -> Result<std::thread::JoinHandle<()>> {
    let parsed = parse_shortcut(&cfg.shortcut)?;
    let repeat_ms = cfg.repeat_debounce_ms;
    let repeat = Duration::from_millis(repeat_ms.max(1));

    let mode = cfg.mode;
    let state = Arc::new(Mutex::new(HotkeyState {
        trigger_pressed: false,
        ctrl_down: false,
        alt_down: false,
        shift_down: false,
        meta_down: false,
        last_press: None,
        last_release: None,
        last_toggle: None,
    }));
    let state = Arc::clone(&state);

    let handle = std::thread::spawn(move || {
        if let Err(err) = listen(move |event: Event| {
            let key = match event.event_type {
                EventType::KeyPress(key) => Some((key, true)),
                EventType::KeyRelease(key) => Some((key, false)),
                _ => None,
            };

            if let Some((key, pressed)) = key {
                let mut emitted = None;

                let mut state = match state.lock() {
                    Ok(state) => state,
                    Err(err) => err.into_inner(),
                };

                match key {
                    k if is_ctrl(k) => state.ctrl_down = pressed,
                    k if is_alt(k) => state.alt_down = pressed,
                    k if is_shift(k) => state.shift_down = pressed,
                    k if is_meta(k) => state.meta_down = pressed,
                    _ => {}
                }

                if key == parsed.key {
                    match mode {
                        InteractionMode::PushToTalk => {
                            if pressed && !state.trigger_pressed {
                                if should_accept_hotkey_event(
                                    &mut state.last_press,
                                    repeat,
                                    "press",
                                ) && mods_match(&parsed, &state)
                                {
                                    state.trigger_pressed = true;
                                    log::debug!(
                                        "push-to-talk keydown accepted for shortcut {:?}",
                                        parsed
                                    );
                                    emitted = Some(AppEvent::Start);
                                }
                            } else if !pressed && state.trigger_pressed {
                                if should_accept_hotkey_event(
                                    &mut state.last_release,
                                    repeat,
                                    "release",
                                ) {
                                    state.trigger_pressed = false;
                                    log::debug!(
                                        "push-to-talk keyup accepted for shortcut {:?}",
                                        parsed
                                    );
                                    emitted = Some(AppEvent::Stop);
                                }
                            }
                        }
                        InteractionMode::Toggle => {
                            if pressed && !state.trigger_pressed && should_accept_hotkey_event(
                                &mut state.last_toggle,
                                repeat,
                                "toggle",
                            ) && mods_match(&parsed, &state)
                            {
                                state.trigger_pressed = true;
                                log::debug!("toggle shortcut accepted: {:?}", parsed);
                                emitted = Some(AppEvent::Toggle);
                            } else if !pressed {
                                state.trigger_pressed = false;
                            }
                        }
                    }
                }

                if let Some(event) = emitted {
                    let _ = tx.send(event);
                }
            }
        }) {
            log::warn!("hotkey listener ended unexpectedly: {err:?}");
        }
    });

    Ok(handle)
}

fn should_accept_hotkey_event(last: &mut Option<Instant>, debounce: Duration, kind: &str) -> bool {
    let now = Instant::now();
    let allowed = match last {
        Some(previous) => now.duration_since(*previous) >= debounce,
        None => true,
    };

    if allowed {
        *last = Some(now);
        return true;
    }

    log::debug!("hotkey {kind} ignored by debounce window");
    false
}

struct HotkeyState {
    trigger_pressed: bool,
    ctrl_down: bool,
    alt_down: bool,
    shift_down: bool,
    meta_down: bool,
    last_press: Option<Instant>,
    last_release: Option<Instant>,
    last_toggle: Option<Instant>,
}

fn mods_match(
    sc: &ParsedShortcut,
    state: &HotkeyState,
) -> bool {
    (!sc.ctrl || state.ctrl_down)
        && (!sc.alt || state.alt_down)
        && (!sc.shift || state.shift_down)
        && (!sc.meta || state.meta_down)
}

fn parse_shortcut(raw: &str) -> Result<ParsedShortcut> {
    let mut out = ParsedShortcut {
        key: Key::Space,
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
    };

    for token in raw.split('+').map(|s| s.trim().to_ascii_lowercase()) {
        if token.is_empty() {
            continue;
        }
        match token.as_str() {
            "ctrl" | "control" => out.ctrl = true,
            "alt" => out.alt = true,
            "shift" => out.shift = true,
            "cmd" | "meta" | "command" | "win" => out.meta = true,
            "space" => out.key = Key::Space,
            "escape" | "esc" => out.key = Key::Escape,
            "enter" => out.key = Key::Return,
            "tab" => out.key = Key::Tab,
            "caps" => out.key = Key::CapsLock,
            v => {
                if v.len() == 1 {
                    let ch = v.as_bytes()[0] as char;
                    out.key = map_char_key(ch)
                        .ok_or_else(|| anyhow!("unsupported key '{ch}' in shortcut"))?;
                } else {
                    return Err(anyhow!("unsupported token '{v}' in shortcut"));
                }
            }
        }
    }

    Ok(out)
}

fn map_char_key(ch: char) -> Option<Key> {
    Some(match ch {
        'a' => Key::KeyA,
        'b' => Key::KeyB,
        'c' => Key::KeyC,
        'd' => Key::KeyD,
        'e' => Key::KeyE,
        'f' => Key::KeyF,
        'g' => Key::KeyG,
        'h' => Key::KeyH,
        'i' => Key::KeyI,
        'j' => Key::KeyJ,
        'k' => Key::KeyK,
        'l' => Key::KeyL,
        'm' => Key::KeyM,
        'n' => Key::KeyN,
        'o' => Key::KeyO,
        'p' => Key::KeyP,
        'q' => Key::KeyQ,
        'r' => Key::KeyR,
        's' => Key::KeyS,
        't' => Key::KeyT,
        'u' => Key::KeyU,
        'v' => Key::KeyV,
        'w' => Key::KeyW,
        'x' => Key::KeyX,
        'y' => Key::KeyY,
        'z' => Key::KeyZ,
        '0' => Key::Num0,
        '1' => Key::Num1,
        '2' => Key::Num2,
        '3' => Key::Num3,
        '4' => Key::Num4,
        '5' => Key::Num5,
        '6' => Key::Num6,
        '7' => Key::Num7,
        '8' => Key::Num8,
        '9' => Key::Num9,
        _ => return None,
    })
}

fn is_ctrl(k: Key) -> bool {
    matches!(k, Key::ControlLeft | Key::ControlRight)
}

fn is_alt(k: Key) -> bool {
    matches!(k, Key::Alt | Key::AltGr)
}

fn is_shift(k: Key) -> bool {
    matches!(k, Key::ShiftLeft | Key::ShiftRight)
}

fn is_meta(k: Key) -> bool {
    matches!(k, Key::MetaLeft | Key::MetaRight)
}
