use crate::config::{AudioCueConfig, AppConfig};
use crate::contracts::events::PipelinePhase;
use crate::contracts::status::SessionStatus;
use rodio::Source;
use std::path::PathBuf;
use std::time::Duration;
use std::sync::mpsc;
use std::thread;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CueKind {
    Start,
    Stop,
    Error,
}

#[derive(Clone)]
pub struct CuePlayer {
    sender: mpsc::Sender<CueKind>,
}

impl CuePlayer {
    pub fn new(cfg: &AudioCueConfig, config: &AppConfig) -> Self {
        let enabled = cfg.enabled;
        let volume = cfg.volume;
        let start_sound = config.resolve_sound_path(cfg.start_sound.as_ref());
        let stop_sound = config.resolve_sound_path(cfg.stop_sound.as_ref());
        let error_sound = config.resolve_sound_path(cfg.error_sound.as_ref());
        let (tx, rx) = mpsc::channel::<CueKind>();

        thread::spawn(move || {
            let player = SerializedCuePlayer {
                enabled,
                volume,
                start_sound,
                stop_sound,
                error_sound,
            };

            while let Ok(kind) = rx.recv() {
                match kind {
                    CueKind::Start => player.play(CueKind::Start),
                    CueKind::Stop => player.play(CueKind::Stop),
                    CueKind::Error => player.play(CueKind::Error),
                }
            }
        });

        Self { sender: tx }
    }

    pub fn play_start(&self) {
        log::debug!("queue start cue");
        let _ = self.sender.send(CueKind::Start);
    }

    pub fn play_stop(&self) {
        log::debug!("queue stop cue");
        let _ = self.sender.send(CueKind::Stop);
    }

    pub fn play_error(&self) {
        log::debug!("queue error cue");
        let _ = self.sender.send(CueKind::Error);
    }

    pub fn status_to_cue(status: &SessionStatus) -> Option<CueKind> {
        if status.source == "start_requested" {
            return Some(CueKind::Start);
        }

        if matches!(
            status.source.as_str(),
            "push_release_stop" | "toggle_press_stop"
        ) {
            return Some(CueKind::Stop);
        }

        if status.state == PipelinePhase::Error {
            return Some(CueKind::Error);
        }

        if status.source.contains("failed") || status.source.contains("queue_saturated") {
            return Some(CueKind::Error);
        }

        None
    }
}

#[derive(Clone)]
struct SerializedCuePlayer {
    enabled: bool,
    volume: f32,
    start_sound: Option<PathBuf>,
    stop_sound: Option<PathBuf>,
    error_sound: Option<PathBuf>,
}

impl SerializedCuePlayer {
    fn play(&self, kind: CueKind) {
        if !self.enabled {
            log::debug!("audio cues disabled");
            return;
        }

        match kind {
            CueKind::Start => self.play_sound(self.start_sound.clone(), 880),
            CueKind::Stop => self.play_sound(self.stop_sound.clone(), 1040),
            CueKind::Error => self.play_sound(self.error_sound.clone(), 220),
        }
    }

    fn play_sound(&self, path: Option<PathBuf>, fallback_hz: u32) {
        if let Some(path) = path {
            if let Ok(file) = std::fs::File::open(&path) {
                if let Ok((stream, handle)) = rodio::OutputStream::try_default() {
                    if let Ok(decoder) = rodio::Decoder::new(std::io::BufReader::new(file)) {
                        if let Some(sink) = rodio::Sink::try_new(&handle).ok() {
                            log::debug!("playing cue {}", path.display());
                            sink.set_volume(self.volume);
                            sink.append(decoder);
                            sink.sleep_until_end();
                            drop(stream);
                            return;
                        }
                        log::warn!("failed to create cue sink for {}", path.display());
                        drop(stream);
                        return fallback_tone(fallback_hz, self.volume);
                    }
                    log::warn!("failed to decode cue {}", path.display());
                    drop(stream);
                    return fallback_tone(fallback_hz, self.volume);
                }
                log::warn!("failed to open output stream for {}", path.display());
                return fallback_tone(fallback_hz, self.volume);
            }
            log::warn!("failed to open cue file {}", path.display());
            return fallback_tone(fallback_hz, self.volume);
        }

        log::debug!("using fallback tone {}hz", fallback_hz);
        fallback_tone(fallback_hz, self.volume);
    }
}

fn fallback_tone(freq: u32, volume: f32) {
    if let Ok((stream, handle)) = rodio::OutputStream::try_default() {
        let sink = rodio::Sink::try_new(&handle).ok();
        if let Some(sink) = sink {
            let tone = rodio::source::SineWave::new(freq as f32)
                .take_duration(Duration::from_millis(120))
                .amplify(volume);
            sink.append(tone);
            sink.sleep_until_end();
        }
        drop(stream);
    }
}
