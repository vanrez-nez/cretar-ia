#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppPhase {
    Idle,
    Recording,
    Sending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    Start,
    Stop,
    Toggle,
    Quit,
}

#[derive(Debug)]
pub enum WorkEvent {
    Completed,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppRuntimeStatus {
    Idle,
    Recording,
    Sending,
    Success,
    Error,
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct RuntimeSessionState {
    pub phase: AppPhase,
    pub status: AppRuntimeStatus,
    pub config_ready: bool,
    pub last_error: Option<String>,
}

impl RuntimeSessionState {
    pub fn new() -> Self {
        Self {
            phase: AppPhase::Idle,
            status: AppRuntimeStatus::Idle,
            config_ready: false,
            last_error: None,
        }
    }

    pub fn set_config_ready(&mut self) {
        self.config_ready = true;
        self.clear_error();
        self.status = AppRuntimeStatus::Idle;
    }

    pub fn start_recording(&mut self) {
        self.phase = AppPhase::Recording;
        self.status = AppRuntimeStatus::Recording;
        self.clear_error();
    }

    pub fn stop_recording(&mut self) {
        self.phase = AppPhase::Sending;
        self.status = AppRuntimeStatus::Sending;
        self.clear_error();
    }

    pub fn complete_send(&mut self) {
        self.phase = AppPhase::Idle;
        self.status = AppRuntimeStatus::Success;
        self.clear_error();
    }

    pub fn fail<S: Into<String>>(&mut self, error: S) {
        self.phase = AppPhase::Idle;
        self.status = AppRuntimeStatus::Error;
        self.last_error = Some(error.into());
    }

    pub fn request_shutdown(&mut self) {
        self.phase = AppPhase::Idle;
        self.status = AppRuntimeStatus::Shutdown;
        self.last_error = None;
    }

    pub fn reset_to_idle(&mut self) {
        self.phase = AppPhase::Idle;
        self.status = AppRuntimeStatus::Idle;
        self.clear_error();
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    pub fn can_start(&self) -> bool {
        self.phase == AppPhase::Idle
    }

    pub fn can_stop(&self) -> bool {
        self.phase == AppPhase::Recording
    }

}

impl AppRuntimeStatus {
    #[cfg(feature = "tray")]
    pub fn icon_state(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Recording => "recording",
            Self::Sending => "sending",
            Self::Success => "done",
            Self::Error => "error",
            Self::Shutdown => "shutdown",
        }
    }
}

impl Default for RuntimeSessionState {
    fn default() -> Self {
        Self::new()
    }
}
