use crate::config::{InteractionConfig, InteractionMode};
use crate::hotkey::input::traits::HotkeyEdgeEvent;
use anyhow::{anyhow, Result};
use rdev::Key;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
struct ParsedShortcut {
    key: Key,
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
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

pub struct HotkeyInputAdapter {
    mode: InteractionMode,
    parsed: ParsedShortcut,
    debounce: Duration,
    state: HotkeyState,
}

impl HotkeyInputAdapter {
    pub fn new(cfg: InteractionConfig) -> Result<Self> {
        let parsed = parse_shortcut(&cfg.shortcut)?;
        Ok(Self {
            mode: cfg.mode,
            parsed,
            debounce: Duration::from_millis(cfg.repeat_debounce_ms.max(1)),
            state: HotkeyState {
                trigger_pressed: false,
                ctrl_down: false,
                alt_down: false,
                shift_down: false,
                meta_down: false,
                last_press: None,
                last_release: None,
                last_toggle: None,
            },
        })
    }

    pub fn ingest(&mut self, key: Key, pressed: bool) -> Option<HotkeyEdgeEvent> {
        match key {
            k if is_ctrl(k) => {
                self.state.ctrl_down = pressed;
                return None;
            }
            k if is_alt(k) => {
                self.state.alt_down = pressed;
                return None;
            }
            k if is_shift(k) => {
                self.state.shift_down = pressed;
                return None;
            }
            k if is_meta(k) => {
                self.state.meta_down = pressed;
                return None;
            }
            _ => {}
        }

        if key != self.parsed.key {
            return None;
        }

        match self.mode {
            InteractionMode::PushToTalk => {
                if pressed {
                    self.handle_push_press()
                } else {
                    self.handle_push_release()
                }
            }
            InteractionMode::Toggle => {
                if pressed {
                    self.handle_toggle_press()
                } else {
                    self.handle_toggle_release()
                }
            }
        }
    }

    fn handle_push_press(&mut self) -> Option<HotkeyEdgeEvent> {
        if !self.state.trigger_pressed {
            if Self::should_emit(self.debounce, &mut self.state.last_press, "press") && self.mods_match() {
                self.state.trigger_pressed = true;
                return Some(HotkeyEdgeEvent::Pressed);
            }
            return Some(HotkeyEdgeEvent::Ignored);
        }

        Some(HotkeyEdgeEvent::Ignored)
    }

    fn handle_push_release(&mut self) -> Option<HotkeyEdgeEvent> {
        if !self.state.trigger_pressed {
            return Some(HotkeyEdgeEvent::Ignored);
        }

        if Self::should_emit(self.debounce, &mut self.state.last_release, "release") {
            self.state.trigger_pressed = false;
            return Some(HotkeyEdgeEvent::Released);
        }

        Some(HotkeyEdgeEvent::Ignored)
    }

    fn handle_toggle_press(&mut self) -> Option<HotkeyEdgeEvent> {
        if self.state.trigger_pressed {
            return Some(HotkeyEdgeEvent::Repeat);
        }

        if Self::should_emit(self.debounce, &mut self.state.last_toggle, "toggle") && self.mods_match() {
            self.state.trigger_pressed = true;
            return Some(HotkeyEdgeEvent::TogglePressed);
        }
        Some(HotkeyEdgeEvent::Ignored)
    }

    fn handle_toggle_release(&mut self) -> Option<HotkeyEdgeEvent> {
        if self.state.trigger_pressed {
            self.state.trigger_pressed = false;
            self.state.last_release = None;
            return Some(HotkeyEdgeEvent::Ignored);
        }
        Some(HotkeyEdgeEvent::Ignored)
    }

    fn should_emit(debounce: Duration, last: &mut Option<Instant>, label: &str) -> bool {
        let now = Instant::now();
        let allowed = match last {
            Some(prev) => now.duration_since(*prev) >= debounce,
            None => true,
        };

        if allowed {
            *last = Some(now);
            return true;
        }

        if !matches!(label, "release") {
            log::debug!("hotkey {label} ignored by debounce window");
        }
        false
    }

    fn mods_match(&self) -> bool {
        (!self.parsed.ctrl || self.state.ctrl_down)
            && (!self.parsed.alt || self.state.alt_down)
            && (!self.parsed.shift || self.state.shift_down)
            && (!self.parsed.meta || self.state.meta_down)
    }
}

fn parse_shortcut(raw: &str) -> Result<ParsedShortcut> {
    let mut out = ParsedShortcut {
        key: Key::Space,
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
    };

    for token in raw.split('+').map(|value| value.trim().to_ascii_lowercase()) {
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
            value => {
                if value.len() == 1 {
                    let ch = value.as_bytes()[0] as char;
                    out.key = map_char_key(ch)
                        .ok_or_else(|| anyhow!("unsupported key '{ch}' in shortcut"))?;
                } else {
                    return Err(anyhow!("unsupported token '{value}' in shortcut"));
                }
            }
        }
    }

    Ok(out)
}

pub fn validate_shortcut(raw: &str) -> Result<()> {
    parse_shortcut(raw).map(|_| ())
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
