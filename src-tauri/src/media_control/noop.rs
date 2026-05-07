pub struct MediaPauseController;

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

    pub fn resume_after_audio_stopped(&self) -> bool {
        false
    }

    pub fn resume_now(&self) -> bool {
        false
    }
}
