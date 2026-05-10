pub struct MediaPauseController;

#[derive(Debug, Clone, Copy)]
pub struct MediaResumeOutcome {
    pub restored: bool,
    pub resumed: bool,
}

impl Default for MediaPauseController {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaPauseController {
    pub fn new() -> Self {
        Self
    }

    pub fn pause_for_recording(&self, _input_device: Option<&str>) -> bool {
        log::debug!("media pause: unsupported on this platform");
        false
    }

    pub fn resume_after_audio_stopped(&self) -> MediaResumeOutcome {
        MediaResumeOutcome {
            restored: true,
            resumed: false,
        }
    }

    pub fn resume_now(&self) -> MediaResumeOutcome {
        MediaResumeOutcome {
            restored: true,
            resumed: false,
        }
    }
}
