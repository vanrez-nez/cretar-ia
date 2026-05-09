mod core_audio;
mod playback;
mod route;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use route::RouteRestoreContext;

pub struct MediaPauseController {
    was_playing_before_recording: AtomicBool,
    route_restore: Mutex<Option<RouteRestoreContext>>,
}

impl Default for MediaPauseController {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaPauseController {
    pub fn new() -> Self {
        Self {
            was_playing_before_recording: AtomicBool::new(false),
            route_restore: Mutex::new(None),
        }
    }

    pub fn pause_for_recording(&self, input_device: Option<&str>) -> bool {
        let context = route::capture_restore_context(input_device);
        if let Ok(mut guard) = self.route_restore.lock() {
            *guard = context;
        }

        let paused = playback::pause_if_playing();
        self.was_playing_before_recording
            .store(paused, Ordering::SeqCst);
        paused
    }

    pub fn resume_after_audio_stopped(&self) -> bool {
        if !self.wait_for_output_route_if_needed() {
            log::warn!("media resume: postponed because output route has not restored");
            return false;
        }
        self.resume_now()
    }

    pub fn resume_now(&self) -> bool {
        if !self.wait_for_output_route_if_needed() {
            log::warn!("media resume: postponed because output route has not restored");
            return false;
        }

        if !self
            .was_playing_before_recording
            .swap(false, Ordering::SeqCst)
        {
            log::debug!("media resume: skipped because this app did not pause media");
            return false;
        }

        playback::resume_if_paused_by_us()
    }

    fn wait_for_output_route_if_needed(&self) -> bool {
        let context = self
            .route_restore
            .lock()
            .ok()
            .and_then(|guard| guard.clone());

        let Some(context) = context else {
            return true;
        };
        let restored = route::wait_for_restore(context);
        if restored {
            if let Ok(mut guard) = self.route_restore.lock() {
                *guard = None;
            }
        }
        restored
    }
}
