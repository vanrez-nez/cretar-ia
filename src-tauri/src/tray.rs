use crate::audio::{available_input_device_names, effective_input_device_name};
use crate::config::AppConfig;
use crate::contracts::events::PipelinePhase;
use crate::contracts::status::SessionStatus;
use crate::i18n;
#[cfg(feature = "settings-ui")]
use crate::model_health::{ModelHealthCache, ModelHealthStatus, ModelHealthView};
use anyhow::{Context, Result};
use resvg::{tiny_skia, usvg};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
#[cfg(feature = "settings-ui")]
use tauri::menu::Submenu;
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Wry};
#[cfg(feature = "settings-ui")]
use tauri::Manager;

pub const MENU_DEVICE_PREFIX: &str = "input-device:";
pub const MENU_DEVICE_DEFAULT: &str = "input-device:system-default";
pub const MENU_MODEL_PREFIX: &str = "model:";
pub const MENU_MODEL_FORMATTING_DISABLE: &str = "model-formatting:disable";
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
    let menu = build_menu(app, cfg)?;
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

    pub fn refresh_menu(&self, app: &AppHandle, config: &AppConfig) {
        log::info!("tray menu rebuild requested");
        match build_menu(app, config) {
            Ok(menu) => {
                if let Err(err) = self.icon.set_menu(Some(menu)) {
                    log::warn!("failed to refresh tray menu: {err}");
                } else {
                    log::info!("tray menu rebuild applied");
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

fn build_menu(app: &AppHandle, config: &AppConfig) -> Result<Menu<Wry>> {
    log::info!(
        "tray menu build config fingerprint={}",
        serde_json::json!({
            "language": config.ui.language,
            "input_device": config.audio.input_device,
            "formatting_enabled": config.models.formatting_enabled,
        })
    );
    let menu = Menu::new(app)?;
    let settings = MenuItem::with_id(app, MENU_SETTINGS, i18n::t_config(&config, "tray.settings"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, i18n::t_config(&config, "tray.quit"), true, None::<&str>)?;
    let separator_after_settings = PredefinedMenuItem::separator(app)?;
    #[cfg(feature = "settings-ui")]
    let separator_after_models = PredefinedMenuItem::separator(app)?;
    let separator_before_quit = PredefinedMenuItem::separator(app)?;
    let devices_label = MenuItem::with_id(app, "input-device:label", i18n::t_config(&config, "tray.inputDevices"), false, None::<&str>)?;

    menu.append(&settings)?;
    menu.append(&separator_after_settings)?;
    #[cfg(feature = "settings-ui")]
    {
        menu.append(&build_models_submenu(app, config)?)?;
        menu.append(&separator_after_models)?;
    }
    menu.append(&devices_label)?;

    let device_names = available_input_device_names();
    let selected_device = selected_input_device(config);
    let configured_device = configured_input_device(config);
    let default_checked = configured_device.is_none();
    let default_item = CheckMenuItem::with_id(
        app,
        MENU_DEVICE_DEFAULT,
        i18n::t_config(&config, "common.systemDefault"),
        true,
        default_checked,
        None::<&str>,
    )?;
    menu.append(&default_item)?;

    if device_names.is_empty() {
        let empty = MenuItem::with_id(app, "input-device:none", i18n::t_config(&config, "tray.noInputDevices"), false, None::<&str>)?;
        menu.append(&empty)?;
    } else {
        for device_name in device_names.iter() {
            let checked = configured_device.is_some()
                && selected_device
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

#[cfg(feature = "settings-ui")]
fn build_models_submenu(app: &AppHandle, config: &AppConfig) -> Result<Submenu<Wry>> {
    let submenu = Submenu::with_id(app, "models", i18n::t_config(config, "tray.models"), true)?;
    let transcript_label = MenuItem::with_id(app, "model:transcript-label", i18n::t_config(config, "models.sttTitle"), false, None::<&str>)?;
    let transform_label = MenuItem::with_id(app, "model:transform-label", i18n::t_config(config, "models.formattingTitle"), false, None::<&str>)?;
    submenu.append(&transcript_label)?;
    append_model_items(app, &submenu, config, "stt")?;
    submenu.append(&PredefinedMenuItem::separator(app)?)?;
    submenu.append(&transform_label)?;
    append_model_items(app, &submenu, config, "formatting")?;
    let disable_title = if config.models.formatting_enabled {
        i18n::t_config(config, "models.disableTransform")
    } else {
        format!("✓ {}", i18n::t_config(config, "models.disableTransform"))
    };
    let disable_transform = MenuItem::with_id(
        app,
        MENU_MODEL_FORMATTING_DISABLE,
        disable_title,
        true,
        None::<&str>,
    )?;
    submenu.append(&disable_transform)?;
    Ok(submenu)
}

#[cfg(feature = "settings-ui")]
fn append_model_items(app: &AppHandle, menu: &Submenu<Wry>, config: &AppConfig, role: &str) -> Result<()> {
    let models = tray_model_snapshot(app);
    let role_models = models
        .iter()
        .filter(|model| model.role == role)
        .collect::<Vec<_>>();
    if role_models.is_empty() {
        let empty = MenuItem::with_id(
            app,
            format!("model:{role}:empty"),
            i18n::t_config(config, "tray.noModels"),
            false,
            None::<&str>,
        )?;
        menu.append(&empty)?;
        return Ok(());
    }

    for model in role_models {
        let item = MenuItem::with_id(
            app,
            format!("{MENU_MODEL_PREFIX}{role}:{}", model.id),
            model_menu_title(model, config),
            true,
            None::<&str>,
        )?;
        menu.append(&item)?;
    }
    Ok(())
}

#[cfg(feature = "settings-ui")]
fn tray_model_snapshot(app: &AppHandle) -> Vec<ModelHealthView> {
    app.try_state::<ModelHealthCache>()
        .map(|cache| cache.snapshot())
        .unwrap_or_default()
}

#[cfg(feature = "settings-ui")]
fn model_menu_title(model: &ModelHealthView, config: &AppConfig) -> String {
    let is_selected = model.is_active
        && !(model.role == "formatting" && !config.models.formatting_enabled);
    let prefix = match (model.health, is_selected) {
        (ModelHealthStatus::Unhealthy, _) => "⚠ ",
        (_, true) => "✓ ",
        (ModelHealthStatus::Healthy, false) => "",
        (ModelHealthStatus::Unknown, false) => "",
    };
    format!("{prefix}{} ({})", model.display_name, model.provider_name)
}

fn selected_input_device(config: &AppConfig) -> Option<String> {
    effective_input_device_name(
        config.audio.input_device.as_deref(),
        config.audio.auto_switch_to_primary_device,
    )
}

fn configured_input_device(config: &AppConfig) -> Option<String> {
    config.audio.input_device.clone()
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
