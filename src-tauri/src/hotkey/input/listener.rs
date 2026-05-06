use crate::config::InteractionConfig;
use crate::recording::command_bus::CommandBusTx;
use crate::hotkey::input::traits::HotkeyInputCoordinator;
use crate::hotkey::input::stabilizer::HotkeyInputAdapter;
use anyhow::Result;
use std::thread::JoinHandle;

use super::traits::HotkeyInputListener;

pub struct RdevHotkeyListener;

impl HotkeyInputListener for RdevHotkeyListener {
    fn start(cfg: InteractionConfig, tx: CommandBusTx) -> Result<JoinHandle<()>> {
        start_listener(cfg, tx)
    }
}

pub fn start_listener(
    cfg: InteractionConfig,
    tx: CommandBusTx,
) -> Result<JoinHandle<()>> {
    #[cfg(target_os = "macos")]
    {
        return macos::start_listener(cfg, tx);
    }

    #[cfg(not(target_os = "macos"))]
    {
        start_rdev_listener(cfg, tx)
    }
}

#[cfg(not(target_os = "macos"))]
fn start_rdev_listener(
    cfg: InteractionConfig,
    tx: CommandBusTx,
) -> Result<JoinHandle<()>> {
    use rdev::{listen, Event, EventType};

    let mut adapter = HotkeyInputAdapter::new(cfg)?;
    let listener_tx = tx.clone();

    let handle = std::thread::spawn(move || {
        if let Err(err) = listen(move |event: Event| {
            if let Some((key, pressed)) = match event.event_type {
                EventType::KeyPress(key) => Some((key, true)),
                EventType::KeyRelease(key) => Some((key, false)),
                _ => None,
            } {
                if let Some(edge) = adapter.ingest(key, pressed) {
                    if let Some(hotkey_event) = HotkeyInputCoordinator::route(edge) {
                        if let Some(worker_event) = listener_tx.send_hotkey(hotkey_event) {
                            let _ = listener_tx.send_worker(worker_event);
                        }
                    }
                }
            }
        }) {
            log::warn!("hotkey listener ended unexpectedly: {err:?}");
        }
    });

    Ok(handle)
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use core_foundation::runloop::CFRunLoop;
    use core_graphics::event::{
        CallbackResult, CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation,
        CGEventTapOptions, CGEventTapPlacement, CGEventType, EventField, KeyCode,
    };
    use rdev::Key;
    use std::sync::{Arc, Mutex};

    struct ListenerState {
        adapter: HotkeyInputAdapter,
        tx: CommandBusTx,
    }

    pub fn start_listener(
        cfg: InteractionConfig,
        tx: CommandBusTx,
    ) -> Result<JoinHandle<()>> {
        let state = Arc::new(Mutex::new(ListenerState {
            adapter: HotkeyInputAdapter::new(cfg)?,
            tx,
        }));

        let handle = std::thread::spawn(move || {
            let callback_state = state.clone();
            let result = CGEventTap::with_enabled(
                CGEventTapLocation::Session,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                vec![
                    CGEventType::KeyDown,
                    CGEventType::KeyUp,
                    CGEventType::FlagsChanged,
                    CGEventType::TapDisabledByTimeout,
                    CGEventType::TapDisabledByUserInput,
                ],
                move |_proxy, event_type, event| {
                    handle_event(&callback_state, event_type, event);
                    CallbackResult::Keep
                },
                CFRunLoop::run_current,
            );

            if result.is_err() {
                log::warn!("failed to install macOS hotkey event tap; accessibility permission may be missing");
            }
        });

        Ok(handle)
    }

    fn handle_event(
        state: &Arc<Mutex<ListenerState>>,
        event_type: CGEventType,
        event: &CGEvent,
    ) {
        if matches!(
            event_type,
            CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
        ) {
            log::warn!("macOS hotkey event tap disabled by system");
            return;
        }

        let Some((key, pressed)) = key_event(event_type, event) else {
            return;
        };

        let Ok(mut state) = state.lock() else {
            log::warn!("macOS hotkey listener state lock poisoned");
            return;
        };

        if let Some(edge) = state.adapter.ingest(key, pressed) {
            if let Some(hotkey_event) = HotkeyInputCoordinator::route(edge) {
                if let Some(worker_event) = state.tx.send_hotkey(hotkey_event) {
                    let _ = state.tx.send_worker(worker_event);
                }
            }
        }
    }

    fn key_event(event_type: CGEventType, event: &CGEvent) -> Option<(Key, bool)> {
        let code = event
            .get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE)
            .try_into()
            .ok()?;
        let key = key_from_macos_code(code);

        match event_type {
            CGEventType::KeyDown => Some((key, true)),
            CGEventType::KeyUp => Some((key, false)),
            CGEventType::FlagsChanged => Some((key, flags_changed_pressed(event))),
            _ => None,
        }
    }

    fn flags_changed_pressed(event: &CGEvent) -> bool {
        static LAST_FLAGS: Mutex<CGEventFlags> = Mutex::new(CGEventFlags::CGEventFlagNull);
        let flags = event.get_flags();

        let Ok(mut last_flags) = LAST_FLAGS.lock() else {
            return true;
        };

        let pressed = flags.bits() > last_flags.bits();
        *last_flags = flags;
        pressed
    }

    fn key_from_macos_code(code: u16) -> Key {
        match code {
            KeyCode::OPTION => Key::Alt,
            KeyCode::RIGHT_OPTION => Key::AltGr,
            KeyCode::DELETE => Key::Backspace,
            KeyCode::CAPS_LOCK => Key::CapsLock,
            KeyCode::CONTROL => Key::ControlLeft,
            KeyCode::RIGHT_CONTROL => Key::ControlRight,
            KeyCode::DOWN_ARROW => Key::DownArrow,
            KeyCode::ESCAPE => Key::Escape,
            KeyCode::F1 => Key::F1,
            KeyCode::F10 => Key::F10,
            KeyCode::F11 => Key::F11,
            KeyCode::F12 => Key::F12,
            KeyCode::F2 => Key::F2,
            KeyCode::F3 => Key::F3,
            KeyCode::F4 => Key::F4,
            KeyCode::F5 => Key::F5,
            KeyCode::F6 => Key::F6,
            KeyCode::F7 => Key::F7,
            KeyCode::F8 => Key::F8,
            KeyCode::F9 => Key::F9,
            KeyCode::LEFT_ARROW => Key::LeftArrow,
            KeyCode::COMMAND => Key::MetaLeft,
            KeyCode::RIGHT_COMMAND => Key::MetaRight,
            KeyCode::RETURN => Key::Return,
            KeyCode::RIGHT_ARROW => Key::RightArrow,
            KeyCode::SHIFT => Key::ShiftLeft,
            KeyCode::RIGHT_SHIFT => Key::ShiftRight,
            KeyCode::SPACE => Key::Space,
            KeyCode::TAB => Key::Tab,
            KeyCode::UP_ARROW => Key::UpArrow,
            KeyCode::ANSI_GRAVE => Key::BackQuote,
            KeyCode::ANSI_1 => Key::Num1,
            KeyCode::ANSI_2 => Key::Num2,
            KeyCode::ANSI_3 => Key::Num3,
            KeyCode::ANSI_4 => Key::Num4,
            KeyCode::ANSI_5 => Key::Num5,
            KeyCode::ANSI_6 => Key::Num6,
            KeyCode::ANSI_7 => Key::Num7,
            KeyCode::ANSI_8 => Key::Num8,
            KeyCode::ANSI_9 => Key::Num9,
            KeyCode::ANSI_0 => Key::Num0,
            KeyCode::ANSI_MINUS => Key::Minus,
            KeyCode::ANSI_EQUAL => Key::Equal,
            KeyCode::ANSI_Q => Key::KeyQ,
            KeyCode::ANSI_W => Key::KeyW,
            KeyCode::ANSI_E => Key::KeyE,
            KeyCode::ANSI_R => Key::KeyR,
            KeyCode::ANSI_T => Key::KeyT,
            KeyCode::ANSI_Y => Key::KeyY,
            KeyCode::ANSI_U => Key::KeyU,
            KeyCode::ANSI_I => Key::KeyI,
            KeyCode::ANSI_O => Key::KeyO,
            KeyCode::ANSI_P => Key::KeyP,
            KeyCode::ANSI_LEFT_BRACKET => Key::LeftBracket,
            KeyCode::ANSI_RIGHT_BRACKET => Key::RightBracket,
            KeyCode::ANSI_A => Key::KeyA,
            KeyCode::ANSI_S => Key::KeyS,
            KeyCode::ANSI_D => Key::KeyD,
            KeyCode::ANSI_F => Key::KeyF,
            KeyCode::ANSI_G => Key::KeyG,
            KeyCode::ANSI_H => Key::KeyH,
            KeyCode::ANSI_J => Key::KeyJ,
            KeyCode::ANSI_K => Key::KeyK,
            KeyCode::ANSI_L => Key::KeyL,
            KeyCode::ANSI_SEMICOLON => Key::SemiColon,
            KeyCode::ANSI_QUOTE => Key::Quote,
            KeyCode::ANSI_BACKSLASH => Key::BackSlash,
            KeyCode::ANSI_Z => Key::KeyZ,
            KeyCode::ANSI_X => Key::KeyX,
            KeyCode::ANSI_C => Key::KeyC,
            KeyCode::ANSI_V => Key::KeyV,
            KeyCode::ANSI_B => Key::KeyB,
            KeyCode::ANSI_N => Key::KeyN,
            KeyCode::ANSI_M => Key::KeyM,
            KeyCode::ANSI_COMMA => Key::Comma,
            KeyCode::ANSI_PERIOD => Key::Dot,
            KeyCode::ANSI_SLASH => Key::Slash,
            KeyCode::FUNCTION => Key::Function,
            value => Key::Unknown(value.into()),
        }
    }
}
