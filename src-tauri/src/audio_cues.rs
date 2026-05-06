use crate::config::{AudioCueConfig, AppConfig};
use crate::contracts::events::PipelinePhase;
use crate::contracts::status::SessionStatus;
use rodio::{OutputStream, OutputStreamHandle, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

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
        log_cue_config(enabled, volume, &start_sound, &stop_sound, &error_sound);
        let (tx, rx) = mpsc::channel::<CueKind>();

        thread::spawn(move || {
            let player = SerializedCuePlayer {
                enabled,
                volume,
                start_sound,
                stop_sound,
                error_sound,
                output: CueOutput::new(),
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

    pub fn run_self_test_if_requested(&self) {
        if std::env::var("CRETAR_IA_AUDIO_CUES_SELF_TEST").ok().as_deref() != Some("1") {
            return;
        }

        log::info!("audio cue self-test requested by CRETAR_IA_AUDIO_CUES_SELF_TEST=1");
        self.play_start();
        self.play_stop();
        self.play_error();
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

struct SerializedCuePlayer {
    enabled: bool,
    volume: f32,
    start_sound: Option<PathBuf>,
    stop_sound: Option<PathBuf>,
    error_sound: Option<PathBuf>,
    output: Option<CueOutput>,
}

struct CueOutput {
    _stream: OutputStream,
    handle: OutputStreamHandle,
}

impl SerializedCuePlayer {
    fn play(&self, kind: CueKind) {
        if !self.enabled {
            log::debug!("audio cue {kind:?} skipped: audio cues disabled");
            return;
        }

        log::debug!("audio cue {kind:?} playback requested");
        match kind {
            CueKind::Start => self.play_sound(self.start_sound.clone(), 880),
            CueKind::Stop => self.play_sound(self.stop_sound.clone(), 1040),
            CueKind::Error => self.play_sound(self.error_sound.clone(), 220),
        }
    }

    fn play_sound(&self, path: Option<PathBuf>, fallback_hz: u32) {
        if let Some(path) = path {
            log::debug!("audio cue opening file {}", path.display());
            match File::open(&path) {
                Ok(file) => match self.output.as_ref() {
                    Some(output) => {
                        match rodio::Decoder::new(BufReader::new(file)) {
                            Ok(decoder) => match rodio::Sink::try_new(&output.handle) {
                                Ok(sink) => {
                            log::debug!("playing cue {}", path.display());
                            sink.set_volume(self.volume);
                            sink.append(decoder);
                            sink.sleep_until_end();
                                    log::debug!("audio cue completed {}", path.display());
                            return;
                        }
                                Err(err) => {
                                    log::warn!(
                                        "failed to create cue sink for {}: {err}",
                                        path.display()
                                    );
                                    fallback_tone(fallback_hz, self.volume);
                                    return;
                                }
                            },
                            Err(err) => {
                                log::warn!("failed to decode cue {}: {err}", path.display());
                                fallback_tone(fallback_hz, self.volume);
                                return;
                            }
                        }
                    }
                    None => {
                        log::warn!(
                            "audio cue output stream unavailable for {}",
                            path.display()
                        );
                        fallback_tone(fallback_hz, self.volume);
                        return;
                    }
                },
                Err(err) => {
                    log::warn!("failed to open cue file {}: {err}", path.display());
                    fallback_tone(fallback_hz, self.volume);
                    return;
                }
            }
        }

        log::debug!("using fallback tone {}hz", fallback_hz);
        fallback_tone(fallback_hz, self.volume);
    }
}

fn fallback_tone(freq: u32, volume: f32) {
    log::debug!("audio cue fallback tone requested: freq={freq}hz volume={volume}");
    match rodio::OutputStream::try_default() {
        Ok((stream, handle)) => {
            let sink = match rodio::Sink::try_new(&handle) {
                Ok(sink) => sink,
                Err(err) => {
                    log::warn!("failed to create fallback cue sink: {err}");
                    drop(stream);
                    return;
                }
            };
            let tone = rodio::source::SineWave::new(freq as f32)
                .take_duration(Duration::from_millis(120))
                .amplify(volume);
            sink.append(tone);
            sink.sleep_until_end();
            log::debug!("audio cue fallback tone completed: freq={freq}hz");
            drop(stream);
        }
        Err(err) => {
            log::warn!("failed to open output stream for fallback cue tone: {err}");
        }
    }
}

impl CueOutput {
    fn new() -> Option<Self> {
        match OutputStream::try_default() {
            Ok((stream, handle)) => {
                log::info!("audio cue output stream initialized");
                Some(Self {
                    _stream: stream,
                    handle,
                })
            }
            Err(err) => {
                log::warn!("failed to initialize audio cue output stream: {err}");
                None
            }
        }
    }
}

fn log_cue_config(
    enabled: bool,
    volume: f32,
    start_sound: &Option<PathBuf>,
    stop_sound: &Option<PathBuf>,
    error_sound: &Option<PathBuf>,
) {
    log::info!("audio cues config: enabled={enabled} volume={volume}");
    log_cue_asset("start", start_sound);
    log_cue_asset("stop", stop_sound);
    log_cue_asset("error", error_sound);
}

fn log_cue_asset(label: &str, path: &Option<PathBuf>) {
    let Some(path) = path else {
        log::info!("audio cue asset {label}: none configured; fallback tone will be used");
        return;
    };

    match std::fs::metadata(path) {
        Ok(metadata) => {
            log::info!(
                "audio cue asset {label}: path={} exists=true bytes={}",
                path.display(),
                metadata.len()
            );
        }
        Err(err) => {
            log::warn!(
                "audio cue asset {label}: path={} exists=false metadata_error={err}",
                path.display()
            );
            return;
        }
    }

    match File::open(path).and_then(|file| {
        rodio::Decoder::new(BufReader::new(file))
            .map(|_| ())
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
    }) {
        Ok(()) => log::debug!("audio cue asset {label}: decoder validation succeeded"),
        Err(err) => log::warn!(
            "audio cue asset {label}: decoder validation failed for {}: {err}",
            path.display()
        ),
    }
}
