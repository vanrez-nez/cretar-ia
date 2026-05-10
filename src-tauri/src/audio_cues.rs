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
    sender: mpsc::Sender<CueRequest>,
}

struct CueRequest {
    kind: CueKind,
    completion: Option<mpsc::Sender<bool>>,
}

impl CuePlayer {
    pub fn new(cfg: &AudioCueConfig, config: &AppConfig) -> Self {
        let enabled = cfg.enabled;
        let volume = cfg.volume;
        let start_sound = config.resolve_sound_path(cfg.start_sound.as_ref());
        let stop_sound = config.resolve_sound_path(cfg.stop_sound.as_ref());
        let error_sound = config.resolve_sound_path(cfg.error_sound.as_ref());
        log_cue_config(enabled, volume, &start_sound, &stop_sound, &error_sound);
        let (tx, rx) = mpsc::channel::<CueRequest>();

        thread::spawn(move || {
            let player = SerializedCuePlayer {
                enabled,
                volume,
                start_sound: CueAsset::load("start", start_sound, 880),
                stop_sound: CueAsset::load("stop", stop_sound, 1040),
                error_sound: CueAsset::load("error", error_sound, 220),
                output: if enabled { CueOutput::new() } else { None },
            };

            while let Ok(request) = rx.recv() {
                let played = match request.kind {
                    CueKind::Start => player.play(CueKind::Start, request.completion.is_some()),
                    CueKind::Stop => player.play(CueKind::Stop, request.completion.is_some()),
                    CueKind::Error => player.play(CueKind::Error, request.completion.is_some()),
                };
                if let Some(completion) = request.completion {
                    if completion.send(played).is_err() {
                        log::debug!("audio cue completion receiver dropped");
                    }
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
        self.queue(CueKind::Start);
    }

    pub fn play_start_and_wait(&self) -> bool {
        log::debug!("queue start cue and wait for completion");
        self.queue_and_wait(CueKind::Start)
    }

    pub fn play_stop(&self) {
        log::debug!("queue stop cue");
        self.queue(CueKind::Stop);
    }

    pub fn play_error(&self) {
        log::debug!("queue error cue");
        self.queue(CueKind::Error);
    }

    pub fn status_to_cue(status: &SessionStatus) -> Option<CueKind> {
        if status.source == "start_requested" {
            return Some(CueKind::Start);
        }

        if matches!(
            status.source.as_str(),
            "push_release_stop" | "toggle_press_stop" | "audio_started_stop_requested"
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

    fn queue(&self, kind: CueKind) {
        let _ = self.sender.send(CueRequest {
            kind,
            completion: None,
        });
    }

    fn queue_and_wait(&self, kind: CueKind) -> bool {
        let (tx, rx) = mpsc::channel();
        if self
            .sender
            .send(CueRequest {
                kind,
                completion: Some(tx),
            })
            .is_err()
        {
            log::warn!("failed to queue waitable audio cue {kind:?}");
            return false;
        }

        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(played) => played,
            Err(err) => {
                log::warn!("waitable audio cue {kind:?} did not complete: {err}");
                false
            }
        }
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
    Silent {
        label: &'static str,
    },
}

impl SerializedCuePlayer {
    fn play(&self, kind: CueKind, wait: bool) -> bool {
        if !self.enabled {
            log::debug!("audio cue {kind:?} skipped: audio cues disabled");
            return true;
        }

        log::debug!("audio cue {kind:?} playback requested");
        let asset = match kind {
            CueKind::Start => &self.start_sound,
            CueKind::Stop => &self.stop_sound,
            CueKind::Error => &self.error_sound,
        };
        self.play_asset(asset, wait)
    }

    fn play_asset(&self, asset: &CueAsset, wait: bool) -> bool {
        if let CueAsset::Silent { label } = asset {
            log::debug!("audio cue {label}: silent cue skipped");
            return true;
        }

        let Some(output) = self.output.as_ref() else {
            log::warn!("audio cue output stream unavailable");
            return false;
        };

        let sink = match rodio::Sink::try_new(&output.handle) {
            Ok(sink) => sink,
            Err(err) => {
                log::warn!("failed to create cue sink: {err}");
                return false;
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
            CueAsset::Silent { .. } => {
                return true;
            }
        }
        if wait {
            sink.sleep_until_end();
        } else {
            sink.detach();
        }
        true
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
            log::info!("audio cue {label}: no file configured; cue will be silent");
            return Self::Silent { label };
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
        log::info!("audio cue asset {label}: none configured; cue will be silent");
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
