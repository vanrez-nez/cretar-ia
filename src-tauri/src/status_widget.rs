use crate::contracts::audio_level::AUDIO_SPECTRUM_BANDS;
use crate::contracts::events::PipelinePhase;
use crate::contracts::status::SessionStatus;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewWindow, WebviewWindowBuilder,
    Window,
};

pub const STATUS_WIDGET_LABEL: &str = "status-widget";

const STATUS_WIDGET_EVENT: &str = "status-widget:update";
const STATUS_WIDGET_AUDIO_LEVEL_EVENT: &str = "status-widget:audio-level";
const STATUS_WIDGET_AUDIO_LEVEL_RESET_EVENT: &str = "status-widget:audio-level-reset";
const COLLAPSED_WIDTH: f64 = 40.0;
const COLLAPSED_HEIGHT: f64 = 5.0;
const EXPANDED_WIDTH: f64 = 60.0;
const EXPANDED_HEIGHT: f64 = 32.0;
const BOTTOM_MARGIN: f64 = 16.0;

#[derive(Clone)]
pub struct StatusWidget {
    app: AppHandle,
    state: Arc<Mutex<StatusWidgetState>>,
}

#[derive(Clone, Copy, Debug, Default)]
struct StatusWidgetState {
    hovered: bool,
    active: bool,
}

impl StatusWidgetState {
    const fn expanded(self) -> bool {
        self.hovered || self.active
    }
}

#[derive(Clone, Copy, Debug)]
struct StatusWidgetLayout {
    width: f64,
    height: f64,
}

impl StatusWidgetLayout {
    const fn for_expanded(expanded: bool) -> Self {
        if expanded {
            Self {
                width: EXPANDED_WIDTH,
                height: EXPANDED_HEIGHT,
            }
        } else {
            Self {
                width: COLLAPSED_WIDTH,
                height: COLLAPSED_HEIGHT,
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct StatusWidgetPayload<'a> {
    label: &'static str,
    state: &'static str,
    expanded: bool,
    source: &'a str,
    session_id: u64,
    phase_elapsed_ms: u64,
    mic_active: bool,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct StatusWidgetAudioLevelPayload {
    levels: [f32; AUDIO_SPECTRUM_BANDS],
}

impl StatusWidget {
    pub fn new(app: &AppHandle) -> Self {
        Self {
            app: app.clone(),
            state: Arc::new(Mutex::new(StatusWidgetState::default())),
        }
    }

    pub fn open(&self) {
        match self.ensure_window() {
            Ok(window) => {
                if let Err(err) =
                    self.apply_layout(&window, StatusWidgetLayout::for_expanded(false))
                {
                    log::warn!("failed to apply status widget initial layout: {err}");
                }
                if let Err(err) = window.show() {
                    log::warn!("failed to show status widget window: {err}");
                }
                let configured_window = self.configured_window();
                log::info!(
                    "status widget window ready transparent={} config_backed={}",
                    configured_window
                        .map(|config| config.transparent)
                        .unwrap_or(false),
                    configured_window.is_some()
                );
            }
            Err(err) => log::warn!("failed to open status widget window: {err}"),
        }
    }

    pub fn update(&self, status: &SessionStatus) {
        let payload = StatusWidgetPayload::from(status);
        let expanded = self.set_active(payload.expanded);

        let window = match self.ensure_window() {
            Ok(window) => window,
            Err(err) => {
                log::warn!("status widget update skipped because window is missing: {err}");
                return;
            }
        };

        if let Err(err) = self.apply_layout(&window, StatusWidgetLayout::for_expanded(expanded)) {
            log::warn!("failed to update status widget layout: {err}");
        }

        if let Err(err) = window.emit(STATUS_WIDGET_EVENT, payload) {
            log::warn!("failed to emit status widget update: {err}");
        }
    }

    pub fn set_hovered(&self, hovered: bool) -> tauri::Result<()> {
        let expanded = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.hovered = hovered;
            state.expanded()
        };
        let window = self.ensure_window()?;
        self.apply_layout(&window, StatusWidgetLayout::for_expanded(expanded))
    }

    pub fn emit_audio_levels(&self, levels: [f32; AUDIO_SPECTRUM_BANDS]) {
        let window = match self.ensure_window() {
            Ok(window) => window,
            Err(err) => {
                log::debug!("status widget audio level skipped because window is missing: {err}");
                return;
            }
        };

        let payload = StatusWidgetAudioLevelPayload {
            levels: levels.map(|level| {
                if level.is_finite() {
                    level.clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }),
        };
        if let Err(err) = window.emit(STATUS_WIDGET_AUDIO_LEVEL_EVENT, payload) {
            log::debug!("failed to emit status widget audio level: {err}");
        }
    }

    pub fn reset_audio_level(&self) {
        let window = match self.ensure_window() {
            Ok(window) => window,
            Err(err) => {
                log::debug!("status widget audio reset skipped because window is missing: {err}");
                return;
            }
        };

        if let Err(err) = window.emit(STATUS_WIDGET_AUDIO_LEVEL_RESET_EVENT, ()) {
            log::debug!("failed to emit status widget audio reset: {err}");
        }
    }

    fn ensure_window(&self) -> tauri::Result<WebviewWindow> {
        if let Some(window) = self.app.get_webview_window(STATUS_WIDGET_LABEL) {
            return Ok(window);
        }

        if let Some(config) = self.configured_window() {
            return WebviewWindowBuilder::from_config(&self.app, config)?.build();
        }

        Err(tauri::Error::InvalidWebviewUrl(
            "status-widget window is missing from tauri.conf.json",
        ))
    }

    fn configured_window(&self) -> Option<&tauri::utils::config::WindowConfig> {
        self.app
            .config()
            .app
            .windows
            .iter()
            .find(|config| config.label == STATUS_WIDGET_LABEL)
    }

    fn set_active(&self, active: bool) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.active = active;
        state.expanded()
    }

    fn apply_layout(
        &self,
        window: &WebviewWindow,
        layout: StatusWidgetLayout,
    ) -> tauri::Result<()> {
        let (x, y) = self.position_for(layout);
        window.set_size(LogicalSize::new(layout.width, layout.height))?;
        window.set_position(LogicalPosition::new(x, y))?;
        Ok(())
    }

    fn position_for(&self, layout: StatusWidgetLayout) -> (f64, f64) {
        let Ok(Some(monitor)) = self.app.primary_monitor() else {
            return (0.0, BOTTOM_MARGIN);
        };

        let work_area = monitor.work_area();
        let scale_factor = monitor.scale_factor().max(1.0);
        let width = layout.width * scale_factor;
        let height = layout.height * scale_factor;
        let margin = BOTTOM_MARGIN * scale_factor;

        let x = work_area.position.x as f64 + (work_area.size.width as f64 - width) / 2.0;
        let y = work_area.position.y as f64 + work_area.size.height as f64 - height - margin;

        (x / scale_factor, y / scale_factor)
    }
}

pub fn handle_window_event(window: &Window, event: &tauri::WindowEvent) -> bool {
    if window.label() != STATUS_WIDGET_LABEL {
        return false;
    }

    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        if let Err(err) = window.show() {
            log::warn!("failed to keep status widget visible on close request: {err}");
        }
    }

    true
}

#[tauri::command]
pub fn set_status_widget_hovered(
    window: Window,
    widget: tauri::State<'_, StatusWidget>,
    hovered: bool,
) -> Result<(), String> {
    if window.label() != STATUS_WIDGET_LABEL {
        return Err("status widget hover command called from unexpected window".to_string());
    }

    widget
        .inner()
        .set_hovered(hovered)
        .map_err(|err| err.to_string())
}

impl<'a> From<&'a SessionStatus> for StatusWidgetPayload<'a> {
    fn from(status: &'a SessionStatus) -> Self {
        let (label, state, expanded) = match status.state {
            PipelinePhase::Idle if status.source == "recording_cancelled" => {
                ("Cancelled", "cancelled", false)
            }
            PipelinePhase::Idle => ("Idle", "idle", false),
            PipelinePhase::Starting | PipelinePhase::Recording => ("Recording", "recording", true),
            PipelinePhase::Stopping | PipelinePhase::Processing => {
                ("Transcribing", "transcribing", true)
            }
            PipelinePhase::Recovering => ("Recovering", "recovering", true),
            PipelinePhase::Error => ("Error", "error", true),
        };

        Self {
            label,
            state,
            expanded,
            source: &status.source,
            session_id: status.session_id,
            phase_elapsed_ms: status.phase_elapsed_ms,
            mic_active: matches!(status.state, PipelinePhase::Recording),
            error: status.error_code.map(|code| code.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::events::{PipelineMode, PipelinePhase};
    use crate::contracts::status::SessionStatus;

    #[test]
    fn recording_cancelled_idle_status_maps_to_cancelled_widget_state() {
        let status = SessionStatus::with_defaults(
            PipelinePhase::Idle,
            PipelineMode::PushToTalk,
            7,
            11,
            "recording_cancelled".to_string(),
        );

        let payload = StatusWidgetPayload::from(&status);

        assert_eq!(payload.label, "Cancelled");
        assert_eq!(payload.state, "cancelled");
        assert!(!payload.expanded);
        assert!(!payload.mic_active);
        assert_eq!(payload.source, "recording_cancelled");
    }

    #[test]
    fn processing_completed_idle_status_keeps_idle_widget_state() {
        let status = SessionStatus::with_defaults(
            PipelinePhase::Idle,
            PipelineMode::PushToTalk,
            7,
            12,
            "processing_completed".to_string(),
        );

        let payload = StatusWidgetPayload::from(&status);

        assert_eq!(payload.label, "Idle");
        assert_eq!(payload.state, "idle");
        assert!(!payload.expanded);
    }
}
