use crate::contracts::events::PipelinePhase;
use crate::contracts::status::SessionStatus;

const ICON_STATE_IDLE: &str = "idle";
const ICON_STATE_RECORDING: &str = "recording";
const ICON_STATE_SENDING: &str = "sending";
const ICON_STATE_ERROR: &str = "error";
const ICON_STATE_DONE: &str = "done";
const ICON_STATE_SHUTDOWN: &str = "shutdown";

pub fn status_to_icon(status: &SessionStatus) -> &'static str {
    match status.state {
        PipelinePhase::Idle => {
            if status.source == "processing_completed" {
                ICON_STATE_DONE
            } else {
                ICON_STATE_IDLE
            }
        }
        PipelinePhase::Starting => ICON_STATE_RECORDING,
        PipelinePhase::Recording => ICON_STATE_RECORDING,
        PipelinePhase::Stopping => ICON_STATE_IDLE,
        PipelinePhase::Processing => ICON_STATE_IDLE,
        PipelinePhase::Recovering => ICON_STATE_ERROR,
        PipelinePhase::Error => ICON_STATE_ERROR,
    }
}

#[cfg(all(feature = "tray", target_os = "macos"))]
mod tray_impl {
    use super::{
        open_settings_file, ICON_STATE_DONE, ICON_STATE_ERROR, ICON_STATE_IDLE,
        ICON_STATE_RECORDING, ICON_STATE_SENDING, ICON_STATE_SHUTDOWN,
    };
    use crate::audio::{available_input_device_names, effective_input_device_name};
    use crate::config::AppConfig;
    use crate::runtime::compat::RuntimeControlEvent;
    use anyhow::{Context, Result};
    use std::collections::HashMap;
    use std::time::{Duration, Instant};
    use resvg::{tiny_skia, usvg};
    use tao::{
        event::{Event, StartCause},
        event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy},
    };
    use tokio::sync::mpsc::UnboundedSender;
    use tray_icon::{
        menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
        Icon, TrayIcon, TrayIconBuilder,
    };

    const MENU_DEVICE_PREFIX: &str = "input-device:";
    const MENU_SETTINGS: &str = "settings";
    const MENU_QUIT: &str = "quit";

    pub struct TrayController {
        event_loop: EventLoop<TrayEvent>,
        handle: TrayHandle,
        title: String,
        tooltip: String,
        tx: UnboundedSender<RuntimeControlEvent>,
        config_path: String,
        icons: IconSet,
        device_names: Vec<String>,
        selected_device: Option<String>,
        configured_device: Option<String>,
    }

    #[derive(Clone)]
    pub struct TrayHandle {
        proxy: EventLoopProxy<TrayEvent>,
    }

    #[derive(Clone, Debug)]
    enum TrayEvent {
        Status {
            tooltip: String,
            state_name: &'static str,
        },
        PulseRecording,
        Quit,
    }

    impl TrayController {
        pub fn new(
            title: String,
            tooltip: String,
            tx: UnboundedSender<RuntimeControlEvent>,
            config_path: String,
        ) -> Result<Self> {
            let event_loop = EventLoopBuilder::<TrayEvent>::with_user_event().build();
            let handle = TrayHandle {
                proxy: event_loop.create_proxy(),
            };
            Ok(Self {
                event_loop,
                handle,
                title,
                tooltip,
                tx,
                config_path,
                icons: IconSet::new()?,
                device_names: available_input_device_names(),
                selected_device: selected_input_device(),
                configured_device: configured_input_device(),
            })
        }

        pub fn handle(&self) -> TrayHandle {
            self.handle.clone()
        }

        pub fn run(self) {
            let mut tray: Option<TrayIcon> = None;
            let mut recording_pulse = false;
            let mut latest_state = ICON_STATE_IDLE;
            let mut latest_tooltip = self.tooltip.clone();
            let tx = self.tx.clone();
            let config_path = self.config_path.clone();
            let icons = self.icons.clone();
            let mut device_names = self.device_names.clone();
            let mut selected_device = self.selected_device.clone();
            let mut configured_device = self.configured_device.clone();
            let mut device_menu_ids = HashMap::<String, String>::new();
            let mut last_device_menu_refresh = Instant::now();

            self.event_loop.run(move |event, _, control_flow| {
                *control_flow = ControlFlow::Wait;

                match event {
                    Event::NewEvents(StartCause::Init) => {
                        let menu = build_menu(
                            &device_names,
                            selected_device.as_deref(),
                            configured_device.as_deref(),
                            &mut device_menu_ids,
                        );

                        match TrayIconBuilder::new()
                            .with_tooltip(latest_tooltip.clone())
                            .with_icon(icons.icon_for_state(latest_state))
                            .with_icon_as_template(false)
                            .with_menu(Box::new(menu))
                            .build()
                        {
                            Ok(item) => tray = Some(item),
                            Err(err) => {
                                log::error!("failed to create macOS tray icon: {err}");
                                *control_flow = ControlFlow::ExitWithCode(1);
                            }
                        }
                    }
                    Event::UserEvent(TrayEvent::Status {
                        tooltip,
                        state_name,
                    }) => {
                        latest_tooltip = tooltip;
                        latest_state = state_name;
                        recording_pulse = matches!(state_name, ICON_STATE_RECORDING);
                        if let Some(tray) = tray.as_ref() {
                            if let Err(err) = tray.set_tooltip(Some(latest_tooltip.clone())) {
                                log::warn!("failed to set tray tooltip: {err}");
                            }
                            if let Err(err) = tray.set_icon(Some(icons.icon_for_state(latest_state))) {
                                log::warn!("failed to set tray icon for state '{latest_state}': {err}");
                            }
                        }
                    }
                    Event::UserEvent(TrayEvent::PulseRecording) => {
                        recording_pulse = !recording_pulse;
                        let state = if recording_pulse {
                            ICON_STATE_IDLE
                        } else {
                            ICON_STATE_RECORDING
                        };
                        if let Some(tray) = tray.as_ref() {
                            if let Err(err) = tray.set_icon(Some(icons.icon_for_state(state))) {
                                log::warn!("failed to pulse tray icon with state '{state}': {err}");
                            }
                        }
                    }
                    Event::UserEvent(TrayEvent::Quit) => {
                        let _ = tx.send(RuntimeControlEvent::Quit);
                        *control_flow = ControlFlow::Exit;
                    }
                    Event::MainEventsCleared => {
                        if last_device_menu_refresh.elapsed() >= Duration::from_secs(2) {
                            last_device_menu_refresh = Instant::now();
                            let next_device_names = available_input_device_names();
                            let next_selected_device = selected_input_device();
                            let next_configured_device = configured_input_device();
                            if next_device_names != device_names
                                || next_selected_device != selected_device
                                || next_configured_device != configured_device
                            {
                                device_names = next_device_names;
                                selected_device = next_selected_device;
                                configured_device = next_configured_device;
                                if let Some(tray) = tray.as_ref() {
                                    let menu = build_menu(
                                        &device_names,
                                        selected_device.as_deref(),
                                        configured_device.as_deref(),
                                        &mut device_menu_ids,
                                    );
                                    tray.set_menu(Some(Box::new(menu)));
                                }
                            }
                        }

                        while let Ok(event) = MenuEvent::receiver().try_recv() {
                            if event.id() == MENU_SETTINGS {
                                if let Err(err) = open_settings_file(&config_path) {
                                    log::warn!("cannot open settings editor: {err}");
                                }
                            } else if event.id() == MENU_QUIT {
                                let _ = tx.send(RuntimeControlEvent::Quit);
                                *control_flow = ControlFlow::Exit;
                            } else if let Some(device_name) = device_menu_ids.get(event.id().as_ref()) {
                                match save_selected_input_device(device_name) {
                                    Ok(()) => {
                                        log::info!("selected input device from tray: {device_name}");
                                        let _ = tx.send(RuntimeControlEvent::SwitchInputDevice);
                                        *control_flow = ControlFlow::Exit;
                                    }
                                    Err(err) => {
                                        log::warn!("failed to save selected input device '{device_name}': {err}");
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            });
        }
    }

    impl TrayHandle {
        pub fn set_status(&self, text: String, state_name: &'static str) {
            log::info!("tray status ({state_name}): {text}");
            let _ = self.proxy.send_event(TrayEvent::Status {
                tooltip: text,
                state_name,
            });
        }

        pub fn pulse_recording(&self) {
            let _ = self.proxy.send_event(TrayEvent::PulseRecording);
        }
    }

    #[derive(Clone)]
    struct IconSet {
        idle: Icon,
        recording: Icon,
        sending: Icon,
        error: Icon,
        done: Icon,
    }

    impl IconSet {
        fn new() -> Result<Self> {
            let theme = IconTheme::current();
            Ok(Self {
                idle: render_svg_icon(theme.idle())?,
                recording: render_svg_icon(theme.recording())?,
                sending: render_svg_icon(theme.off())?,
                error: render_svg_icon(theme.off())?,
                done: render_svg_icon(theme.idle())?,
            })
        }

        fn icon_for_state(&self, state: &str) -> Icon {
            match state {
                ICON_STATE_RECORDING => self.recording.clone(),
                ICON_STATE_SENDING => self.sending.clone(),
                ICON_STATE_ERROR => self.error.clone(),
                ICON_STATE_DONE => self.done.clone(),
                ICON_STATE_SHUTDOWN => self.idle.clone(),
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

    fn render_svg_icon(svg: &'static [u8]) -> Result<Icon> {
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

        Icon::from_rgba(pixmap.take(), size.width(), size.height())
            .context("failed to build tray icon from rendered SVG")
    }

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

    fn matches_dark_style(value: Option<&str>) -> bool {
        value
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("dark")
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

    fn save_selected_input_device(device_name: &str) -> Result<()> {
        let mut config = AppConfig::load_or_create().context("loading config for input device selection")?;
        config.audio.input_device = Some(device_name.to_string());
        config
            .save_validated_to(AppConfig::config_path())
            .context("saving selected input device")
    }

    fn build_menu(
        device_names: &[String],
        selected_device: Option<&str>,
        configured_device: Option<&str>,
        device_menu_ids: &mut HashMap<String, String>,
    ) -> Menu {
        let menu = Menu::new();
        device_menu_ids.clear();
        let settings = MenuItem::with_id(MENU_SETTINGS, "Settings", true, None);
        let separator_after_settings = PredefinedMenuItem::separator();
        let separator_before_quit = PredefinedMenuItem::separator();
        let quit = MenuItem::with_id(MENU_QUIT, "Quit", true, None);

        if let Err(err) = menu.append(&settings) {
            log::warn!("failed to add settings menu item: {err}");
        }
        if let Err(err) = menu.append(&separator_after_settings) {
            log::warn!("failed to add menu separator: {err}");
        }

        let input_devices_label = MenuItem::with_id("input-device:label", "Input Devices", false, None);
        if let Err(err) = menu.append(&input_devices_label) {
            log::warn!("failed to add input devices menu label: {err}");
        }

        if device_names.is_empty() {
            let empty = MenuItem::with_id("input-device:none", "No input devices found", false, None);
            if let Err(err) = menu.append(&empty) {
                log::warn!("failed to add empty input device menu item: {err}");
            }
        } else {
            for (idx, device_name) in device_names.iter().enumerate() {
                let id = format!("{MENU_DEVICE_PREFIX}{idx}");
                let checked = selected_device
                    .is_some_and(|selected| selected == device_name);
                let item = CheckMenuItem::with_id(
                    id.clone(),
                    device_name,
                    true,
                    checked,
                    None,
                );
                if let Err(err) = menu.append(&item) {
                    log::warn!("failed to add input device menu item '{device_name}': {err}");
                }
                device_menu_ids.insert(id, device_name.clone());
            }
        }

        if let Some(configured) = configured_device {
            let configured_available = device_names.iter().any(|name| name == configured);
            if !configured_available {
                let unavailable = MenuItem::with_id(
                    "input-device:unavailable",
                    configured,
                    false,
                    None,
                );
                if let Err(err) = menu.append(&unavailable) {
                    log::warn!("failed to add unavailable input device menu item: {err}");
                }
            }
        }

        if let Err(err) = menu.append_items(&[&separator_before_quit, &quit]) {
            log::warn!("failed to build tray menu: {err}");
        }

        menu
    }
}

#[cfg(all(feature = "tray", not(target_os = "macos")))]
mod tray_impl {
    use super::{
        open_settings_file, ICON_STATE_DONE, ICON_STATE_ERROR, ICON_STATE_IDLE,
        ICON_STATE_RECORDING, ICON_STATE_SENDING, ICON_STATE_SHUTDOWN,
    };
    use crate::runtime::compat::RuntimeControlEvent;
    use anyhow::Result;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc::UnboundedSender;
    use tray_item::{IconSource, TrayItem};

    pub struct TrayController {
        handle: TrayHandle,
    }

    #[derive(Clone)]
    pub struct TrayHandle {
        item: Arc<Mutex<TrayItem>>,
        recording_pulse: Arc<Mutex<bool>>,
    }

    impl TrayController {
        pub fn new(
            title: String,
            _tooltip: String,
            tx: UnboundedSender<RuntimeControlEvent>,
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
                let _ = tx.send(RuntimeControlEvent::Quit);
                std::process::exit(0);
            });
            Ok(Self {
                handle: TrayHandle {
                    item: Arc::new(Mutex::new(item)),
                    recording_pulse: Arc::new(Mutex::new(false)),
                },
            })
        }

        pub fn handle(&self) -> TrayHandle {
            self.handle.clone()
        }

        pub fn run(self) {}
    }

    impl TrayHandle {
        pub fn set_status(&self, text: String, state_name: &'static str) {
            log::info!("tray status ({state_name}): {text}");
            if let Ok(mut recording_pulse) = self.recording_pulse.lock() {
                *recording_pulse = matches!(state_name, ICON_STATE_RECORDING);
            }
            if let Ok(mut item) = self.item.lock() {
                if let Err(err) = item.set_icon(icon_for_state(state_name)) {
                    log::warn!("failed to set tray icon for state '{state_name}': {err}");
                }
            }
        }

        pub fn pulse_recording(&self) {
            let phase = if let Ok(mut recording_pulse) = self.recording_pulse.lock() {
                *recording_pulse = !*recording_pulse;
                if *recording_pulse {
                    ICON_STATE_IDLE
                } else {
                    ICON_STATE_RECORDING
                }
            } else {
                ICON_STATE_RECORDING
            };

            if let Ok(mut item) = self.item.lock() {
                if let Err(err) = item.set_icon(icon_for_state(phase)) {
                    log::warn!("failed to pulse tray icon with state '{phase}': {err}");
                }
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
        if crate::settings_control::notify_settings_focus().is_ok() {
            return Ok(());
        }

        let exe = std::env::current_exe()?;
        let mut command = std::process::Command::new(exe);
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
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(std::io::Error::other)
}

#[cfg(not(feature = "tray"))]
mod tray_impl {
    use anyhow::Result;
    use crate::runtime::compat::RuntimeControlEvent;
    use tokio::sync::mpsc::UnboundedSender;

    pub struct TrayController {
        _status: String,
    }

    #[derive(Clone)]
    pub struct TrayHandle;

    impl TrayController {
        pub fn new(
            _title: String,
            tooltip: String,
            _tx: UnboundedSender<RuntimeControlEvent>,
            _config_path: String,
        ) -> Result<Self> {
            log::info!("tray disabled; status: {}", tooltip);
            Ok(Self { _status: tooltip })
        }

        pub fn handle(&self) -> TrayHandle {
            TrayHandle
        }

        pub fn run(self) {}
    }

    impl TrayHandle {
        pub fn set_status(&self, text: String, _state_name: &'static str) {
            log::info!("tray status: {text}");
        }

        pub fn pulse_recording(&self) {}
    }
}

pub use tray_impl::{TrayController, TrayHandle};
