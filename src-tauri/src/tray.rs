use crate::config::AppEvent;
 use anyhow::Result;
#[cfg(feature = "tray")]
use std::process::Command;
#[cfg(feature = "tray")]
use std::io;
use tokio::sync::mpsc::UnboundedSender;

#[cfg(feature = "tray")]
mod tray_impl {
    use super::*;
    use tray_item::{IconSource, TrayItem};

    const ICON_STATE_IDLE: &str = "idle";
    const ICON_STATE_RECORDING: &str = "recording";
    const ICON_STATE_SENDING: &str = "sending";
    const ICON_STATE_ERROR: &str = "error";
    const ICON_STATE_DONE: &str = "done";
    const ICON_STATE_SHUTDOWN: &str = "shutdown";

    pub struct TrayController {
        item: TrayItem,
        recording_pulse: bool,
    }

    impl TrayController {
        pub fn new(
            title: String,
            _tooltip: String,
            tx: UnboundedSender<AppEvent>,
            config_path: String,
        ) -> Result<Self> {
            let mut item = TrayItem::new(&title, icon_for_state(ICON_STATE_IDLE))?;
            let settings_path = config_path;
            let _ = item.add_menu_item("Settings", {
                move || {
                    if let Err(err) = open_settings_file(&settings_path) {
                        log::warn!("cannot open settings editor: {err}");
                    }
                }
            });
            let _ = item.add_menu_item("Quit", move || {
                let _ = tx.send(AppEvent::Quit);
                std::process::exit(0);
            });
            Ok(Self {
                item,
                recording_pulse: false,
            })
        }

        #[cfg(target_os = "macos")]
        pub fn run(mut self) {
            let inner = self.item.inner_mut();
            inner.display();
        }

        #[cfg(not(target_os = "macos"))]
        pub fn run(self) {}

        #[cfg_attr(target_os = "macos", allow(dead_code))]
        pub fn set_status(&mut self, text: String, state: &str) {
            log::info!("tray status ({state}): {text}");
            self.recording_pulse = matches!(state, ICON_STATE_RECORDING);
            if let Err(err) = self.item.set_icon(icon_for_state(state)) {
                log::warn!("failed to set tray icon for state '{state}': {err}");
            }
        }

        #[cfg_attr(target_os = "macos", allow(dead_code))]
        pub fn pulse_recording(&mut self) {
            self.recording_pulse = !self.recording_pulse;
            let phase = if self.recording_pulse {
                ICON_STATE_SENDING
            } else {
                ICON_STATE_RECORDING
            };

            if let Err(err) = self.item.set_icon(icon_for_state(phase)) {
                log::warn!("failed to pulse tray icon with state '{phase}': {err}");
            }
        }
    }

    fn icon_for_state(state: &str) -> IconSource {
        let icon_name = match state {
            ICON_STATE_RECORDING => ICON_STATE_RECORDING,
            ICON_STATE_SENDING => ICON_STATE_SENDING,
            ICON_STATE_ERROR => "off",
            ICON_STATE_DONE | ICON_STATE_SHUTDOWN => ICON_STATE_IDLE,
            _ => ICON_STATE_IDLE,
        };
        match_icon(icon_name)
    }

    #[cfg(target_os = "macos")]
    fn match_icon(icon_name: &str) -> IconSource {
        let themed_icon = if is_dark_mode() {
            match icon_name {
                "idle" => "idle-dark",
                "recording" => "recording-dark",
                "sending" => "off-dark",
                "off" => "off-dark",
                _ => "idle-dark",
            }
        } else {
            match icon_name {
                "idle" => "idle-light",
                "recording" => "recording-light",
                "sending" => "off-light",
                "off" => "off-light",
                _ => "idle-light",
            }
        };

        IconSource::Data {
            height: 0,
            width: 0,
            data: icon_bytes(themed_icon).to_vec(),
        }
    }

    #[cfg(target_os = "macos")]
    fn is_dark_mode() -> bool {
        let env_value = std::env::var("AppleInterfaceStyle").ok();
        if matches_dark_style(env_value.as_deref()) {
            return true;
        }

        use std::process::Command;
        let Ok(output) = Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
        else {
            return false;
        };

        if !output.status.success() {
            return false;
        }

        if let Ok(style) = std::str::from_utf8(&output.stdout) {
            return matches_dark_style(Some(style));
        }

        false
    }

    #[cfg(target_os = "macos")]
    fn icon_bytes(icon_name: &str) -> &'static [u8] {
        match icon_name {
            "idle-light" => include_bytes!("../icons/idle-light.svg"),
            "idle-dark" => include_bytes!("../icons/idle-dark.svg"),
            "recording-light" => include_bytes!("../icons/recording-light.svg"),
            "recording-dark" => include_bytes!("../icons/recording-dark.svg"),
            "off-light" => include_bytes!("../icons/off-light.svg"),
            "off-dark" => include_bytes!("../icons/off-dark.svg"),
            _ => include_bytes!("../icons/idle-light.svg"),
        }
    }

    #[cfg(target_os = "macos")]
    fn matches_dark_style(value: Option<&str>) -> bool {
        value
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("dark")
    }

    #[cfg(target_os = "linux")]
    fn match_icon(icon_name: &str) -> IconSource {
        let icon_name = match icon_name {
            "recording" => "media-record-symbolic",
            "off" => "audio-input-microphone-muted-symbolic",
            "sending" => "audio-input-microphone-symbolic",
            _ => "audio-input-microphone-symbolic",
        };
        IconSource::Resource(icon_name)
    }

    #[cfg(target_os = "windows")]
    fn match_icon(icon_name: &str) -> IconSource {
        let icon_name = match icon_name {
            "recording" => "recording",
            "off" | "sending" => "off",
            _ => "idle",
        };
        IconSource::Data {
            height: 0,
            width: 0,
            data: icon_bytes(icon_name).to_vec(),
        }
    }

    #[cfg(target_os = "windows")]
    fn icon_bytes(icon_name: &str) -> &'static [u8] {
        match icon_name {
            "recording" => include_bytes!("../icons/recording.svg"),
            "off" => include_bytes!("../icons/off.svg"),
            _ => include_bytes!("../icons/idle.svg"),
        }
    }
}

#[cfg(feature = "tray")]
fn open_settings_file(path: &str) -> std::io::Result<()> {
    #[cfg(feature = "settings-ui")]
    {
        let exe = std::env::current_exe()?;
        let mut command = Command::new(exe);
        command.arg("--settings");
        match command.spawn().map(|_| ()) {
            Ok(()) => return Ok(()),
            Err(err) => {
                log::warn!("settings-ui launch failed: {err}");
                return open_path_with_default_app(path);
            }
        }
    }

    #[cfg(not(feature = "settings-ui"))]
    {
        return open_path_with_default_app(path);
    }
}

#[cfg(feature = "tray")]
fn open_path_with_default_app(path: &str) -> std::io::Result<()> {
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(io::Error::other)
}

#[cfg(not(feature = "tray"))]
mod tray_impl {
    use super::*;

    pub struct TrayController {
        _status: String,
    }

    impl TrayController {
        pub fn new(
            _title: String,
            tooltip: String,
            _tx: UnboundedSender<AppEvent>,
            _config_path: String,
        ) -> Result<Self> {
            log::info!("tray disabled; status: {}", tooltip);
            Ok(Self { _status: tooltip })
        }

        pub fn run(self) {}

        #[cfg(not(target_os = "macos"))]
        pub fn set_status(&mut self, text: String, _state: &str) {
            log::info!("tray status: {text}");
        }

        #[cfg(not(target_os = "macos"))]
        pub fn pulse_recording(&mut self) {}
    }
}

pub use tray_impl::TrayController;
