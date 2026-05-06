use crate::config::{AudioCueConfig, AppConfig};
use crate::contracts::events::PipelinePhase;
use crate::contracts::status::SessionStatus;
use rodio::{buffer::SamplesBuffer, OutputStream, OutputStreamHandle, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
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
                start_sound: CueAsset::load("start", start_sound, 880),
                stop_sound: CueAsset::load("stop", stop_sound, 1040),
                error_sound: CueAsset::load("error", error_sound, 220),
                output: if enabled { CueOutput::new() } else { None },
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
    start_sound: CueAsset,
    stop_sound: CueAsset,
    error_sound: CueAsset,
    output: Option<CueOutput>,
}

struct CueOutput {
    _stream: OutputStream,
    handle: OutputStreamHandle,
}

enum CueAsset {
    Buffered {
        label: &'static str,
        channels: u16,
        sample_rate: u32,
        samples: Arc<Vec<f32>>,
    },
    Tone {
        label: &'static str,
        hz: u32,
    },
}

impl SerializedCuePlayer {
    fn play(&self, kind: CueKind) {
        if !self.enabled {
            log::debug!("audio cue {kind:?} skipped: audio cues disabled");
            return;
        }

        log::debug!("audio cue {kind:?} playback requested");
        let asset = match kind {
            CueKind::Start => &self.start_sound,
            CueKind::Stop => &self.stop_sound,
            CueKind::Error => &self.error_sound,
        };
        self.play_asset(asset);
    }

    fn play_asset(&self, asset: &CueAsset) {
        let Some(output) = self.output.as_ref() else {
            log::warn!("audio cue output stream unavailable");
            return;
        };

        let sink = match rodio::Sink::try_new(&output.handle) {
            Ok(sink) => sink,
            Err(err) => {
                log::warn!("failed to create cue sink: {err}");
                return;
            }
        };

        sink.set_volume(self.volume);
        match asset {
            CueAsset::Buffered {
                label,
                channels,
                sample_rate,
                samples,
                ..
            } => {
                log::debug!("playing buffered cue {label}");
                let source = SamplesBuffer::new(*channels, *sample_rate, samples.as_ref().clone());
                sink.append(source);
            }
            CueAsset::Tone { label, hz } => {
                log::debug!("playing fallback cue {label}: {hz}hz");
                let source = rodio::source::SineWave::new(*hz as f32)
                    .take_duration(Duration::from_millis(120));
                sink.append(source);
            }
        }
        sink.detach();
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

impl CueAsset {
    fn load(label: &'static str, path: Option<PathBuf>, fallback_hz: u32) -> Self {
        let Some(path) = path else {
            log::info!("audio cue {label}: no file configured; using fallback tone");
            return Self::Tone {
                label,
                hz: fallback_hz,
            };
        };

        let file = match File::open(&path) {
            Ok(file) => file,
            Err(err) => {
                log::warn!(
                    "audio cue {label}: failed to open {}; using fallback tone: {err}",
                    path.display()
                );
                return Self::Tone {
                    label,
                    hz: fallback_hz,
                };
            }
        };

        let decoder = match rodio::Decoder::new(BufReader::new(file)) {
            Ok(decoder) => decoder,
            Err(err) => {
                log::warn!(
                    "audio cue {label}: failed to decode {}; using fallback tone: {err}",
                    path.display()
                );
                return Self::Tone {
                    label,
                    hz: fallback_hz,
                };
            }
        };

        let channels = decoder.channels();
        let sample_rate = decoder.sample_rate();
        let samples: Vec<f32> = decoder.convert_samples::<f32>().collect();
        let duration_ms = if channels > 0 && sample_rate > 0 {
            (samples.len() as f64 / channels as f64 / sample_rate as f64 * 1000.0).round() as u64
        } else {
            0
        };
        log::info!(
            "audio cue {label}: buffered {} samples from {} (channels={} sample_rate={} duration_ms={})",
            samples.len(),
            path.display(),
            channels,
            sample_rate,
            duration_ms
        );

        Self::Buffered {
            label,
            channels,
            sample_rate,
            samples: Arc::new(samples),
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
