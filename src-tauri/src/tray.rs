use crate::audio::{available_input_device_names, effective_input_device_name};
use crate::config::AppConfig;
use crate::contracts::events::PipelinePhase;
use crate::contracts::status::SessionStatus;
use anyhow::{Context, Result};
use resvg::{tiny_skia, usvg};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Wry};

pub const MENU_DEVICE_PREFIX: &str = "input-device:";
pub const MENU_SETTINGS: &str = "settings";
pub const MENU_QUIT: &str = "quit";

const ICON_STATE_IDLE: &str = "idle";
const ICON_STATE_RECORDING: &str = "recording";
const ICON_STATE_SENDING: &str = "sending";
const ICON_STATE_ERROR: &str = "error";
const ICON_STATE_DONE: &str = "done";
const ICON_STATE_SHUTDOWN: &str = "shutdown";

#[derive(Clone)]
pub struct AppTray {
    icon: TrayIcon<Wry>,
    icons: IconSet,
}

pub fn create_tray(app: &AppHandle, cfg: &AppConfig) -> Result<AppTray> {
    let icons = IconSet::new()?;
    let menu = build_menu(app)?;
    let icon = TrayIconBuilder::with_id("main")
        .tooltip(&cfg.tray.tooltip.idle)
        .icon(icons.icon_for_state(ICON_STATE_IDLE))
        .icon_as_template(false)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .build(app)?;

    Ok(AppTray { icon, icons })
}

impl AppTray {
    pub fn set_status(&self, text: String, state_name: &'static str) {
        log::info!("tray status ({state_name}): {text}");
        if let Err(err) = self.icon.set_tooltip(Some(text)) {
            log::warn!("failed to set tray tooltip: {err}");
        }
        if let Err(err) = self.icon.set_icon(Some(self.icons.icon_for_state(state_name))) {
            log::warn!("failed to set tray icon for state '{state_name}': {err}");
        }
    }

    pub fn pulse_recording(&self) {
        if let Err(err) = self.icon.set_icon(Some(self.icons.icon_for_state(ICON_STATE_RECORDING))) {
            log::warn!("failed to pulse tray icon: {err}");
        }
    }

    pub fn refresh_menu(&self, app: &AppHandle) {
        match build_menu(app) {
            Ok(menu) => {
                if let Err(err) = self.icon.set_menu(Some(menu)) {
                    log::warn!("failed to refresh tray menu: {err}");
                }
            }
            Err(err) => log::warn!("failed to rebuild tray menu: {err}"),
        }
    }
}

pub fn status_to_icon(status: &SessionStatus) -> &'static str {
    match status.state {
        PipelinePhase::Idle => {
            if status.source == "processing_completed" {
                ICON_STATE_DONE
            } else {
                ICON_STATE_IDLE
            }
        }
        PipelinePhase::Starting | PipelinePhase::Recording | PipelinePhase::Stopping => {
            ICON_STATE_RECORDING
        }
        PipelinePhase::Processing => ICON_STATE_SENDING,
        PipelinePhase::Recovering | PipelinePhase::Error => ICON_STATE_ERROR,
    }
}

pub fn save_selected_input_device(device_name: &str) -> Result<()> {
    let mut config = AppConfig::load_or_create().context("loading config for input device selection")?;
    config.audio.input_device = Some(device_name.to_string());
    config
        .save_validated_to(AppConfig::config_path())
        .context("saving selected input device")
}

fn build_menu(app: &AppHandle) -> Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    let settings = MenuItem::with_id(app, MENU_SETTINGS, "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;
    let separator_after_settings = PredefinedMenuItem::separator(app)?;
    let separator_before_quit = PredefinedMenuItem::separator(app)?;
    let devices_label = MenuItem::with_id(app, "input-device:label", "Input Devices", false, None::<&str>)?;

    menu.append(&settings)?;
    menu.append(&separator_after_settings)?;
    menu.append(&devices_label)?;

    let device_names = available_input_device_names();
    let selected_device = selected_input_device();
    let configured_device = configured_input_device();

    if device_names.is_empty() {
        let empty = MenuItem::with_id(app, "input-device:none", "No input devices found", false, None::<&str>)?;
        menu.append(&empty)?;
    } else {
        for device_name in device_names.iter() {
            let checked = selected_device
                .as_deref()
                .is_some_and(|selected| selected == device_name);
            let item = CheckMenuItem::with_id(
                app,
                format!("{MENU_DEVICE_PREFIX}{device_name}"),
                device_name,
                true,
                checked,
                None::<&str>,
            )?;
            menu.append(&item)?;
        }
    }

    if let Some(configured) = configured_device {
        let configured_available = device_names.iter().any(|name| name == &configured);
        if !configured_available {
            let unavailable = MenuItem::with_id(
                app,
                "input-device:unavailable",
                configured,
                false,
                None::<&str>,
            )?;
            menu.append(&unavailable)?;
        }
    }

    menu.append(&separator_before_quit)?;
    menu.append(&quit)?;
    Ok(menu)
}

fn selected_input_device() -> Option<String> {
    let config = AppConfig::load_or_create().ok()?;
    effective_input_device_name(
        config.audio.input_device.as_deref(),
        config.audio.auto_switch_to_primary_device,
    )
}

fn configured_input_device() -> Option<String> {
    AppConfig::load_or_create()
        .ok()
        .and_then(|config| config.audio.input_device)
}

#[derive(Clone)]
struct IconSet {
    idle: Image<'static>,
    recording: Image<'static>,
    sending: Image<'static>,
    error: Image<'static>,
    done: Image<'static>,
}

impl IconSet {
    fn new() -> Result<Self> {
        let theme = IconTheme::current();
        Ok(Self {
            idle: render_svg_icon(theme.idle())?,
            recording: render_svg_icon(theme.recording())?,
            sending: render_svg_icon(theme.idle())?,
            error: render_svg_icon(theme.off())?,
            done: render_svg_icon(theme.idle())?,
        })
    }

    fn icon_for_state(&self, state: &str) -> Image<'static> {
        match state {
            ICON_STATE_RECORDING => self.recording.clone(),
            ICON_STATE_SENDING => self.sending.clone(),
            ICON_STATE_ERROR => self.error.clone(),
            ICON_STATE_DONE | ICON_STATE_SHUTDOWN => self.done.clone(),
            _ => self.idle.clone(),
        }
    }
}

enum IconTheme {
    Light,
    Dark,
}

impl IconTheme {
    fn current() -> Self {
        if is_dark_mode() {
            Self::Dark
        } else {
            Self::Light
        }
    }

    fn idle(&self) -> &'static [u8] {
        match self {
            Self::Light => include_bytes!("../icons/idle-light.svg"),
            Self::Dark => include_bytes!("../icons/idle-dark.svg"),
        }
    }

    fn recording(&self) -> &'static [u8] {
        match self {
            Self::Light => include_bytes!("../icons/recording-light.svg"),
            Self::Dark => include_bytes!("../icons/recording-dark.svg"),
        }
    }

    fn off(&self) -> &'static [u8] {
        match self {
            Self::Light => include_bytes!("../icons/off-light.svg"),
            Self::Dark => include_bytes!("../icons/off-dark.svg"),
        }
    }
}

fn render_svg_icon(svg: &'static [u8]) -> Result<Image<'static>> {
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(svg, &options).context("failed to parse tray SVG")?;
    let size = tree.size().to_int_size();
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height())
        .context("failed to allocate tray icon pixmap")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );

    Ok(Image::new_owned(pixmap.take(), size.width(), size.height()))
}

fn is_dark_mode() -> bool {
    let env_value = std::env::var("AppleInterfaceStyle").ok();
    if matches_dark_style(env_value.as_deref()) {
        return true;
    }

    let Ok(output) = std::process::Command::new("defaults")
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

fn matches_dark_style(value: Option<&str>) -> bool {
    value
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("dark")
}
