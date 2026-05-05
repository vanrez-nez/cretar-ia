use crate::config::AudioCaptureConfig;
use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleFormat, SampleRate, Stream, StreamConfig};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const TARGET_PEAK: f32 = 0.2;
const MAX_NORMALIZE_GAIN: f32 = 64.0;
const NOISY_EPS: f32 = 1e-7;

pub struct Recorder {
    stream: Stream,
    state: Arc<Mutex<RecorderState>>,
    sample_rate: u32,
    channels: u16,
    out_path: PathBuf,
}

impl Recorder {
    pub fn start(config: &AudioCaptureConfig, base_dir: PathBuf) -> Result<Self> {
        report_runtime_context();
        let host = cpal::default_host();
        let device = select_input_device(&host, config.input_device.as_deref())?;
        let supported = device
            .default_input_config()
            .context("failed reading default input config")?;
        let device_name = device
            .name()
            .unwrap_or_else(|_| "default input device".to_string());

        log::info!(
            "audio device selected: {device_name} format={:?} sample_rate={} channels={}",
            supported.sample_format(),
            supported.sample_rate().0,
            supported.channels()
        );

        let state = Arc::new(Mutex::new(RecorderState::default()));

        let requested = StreamConfig {
            sample_rate: SampleRate(if config.sample_rate == 0 {
                supported.sample_rate().0
            } else {
                config.sample_rate
            }),
            channels: if config.channels == 0 {
                supported.channels()
            } else {
                config.channels
            },
            buffer_size: BufferSize::Default,
        };
        let requested_sample_rate = requested.sample_rate.0;
        let requested_channels = requested.channels;

        let fallback: StreamConfig = supported.clone().into();

        let (stream_cfg, stream) = match build_input_stream_for_format(
            &device,
            supported.sample_format(),
            &requested,
            Arc::clone(&state),
        ) {
            Ok(stream) => (requested, stream),
            Err(err) => {
                log::warn!(
                    "using fallback stream config after requested config failed (sample_rate={}, channels={}): {err}",
                    requested_sample_rate,
                    requested_channels
                );
                (
                    fallback.clone(),
                    build_input_stream_for_format(
                        &device,
                        supported.sample_format(),
                        &fallback,
                        Arc::clone(&state),
                    )?,
                )
            }
        };

        log::info!(
            "audio config: requested sample_rate={} channels={} -> active sample_rate={} channels={}",
            requested_sample_rate,
            requested_channels,
            stream_cfg.sample_rate.0,
            stream_cfg.channels
        );

        stream.play().context("starting audio stream")?;

        std::fs::create_dir_all(&base_dir).context("creating recording directory")?;
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|v| v.as_millis())
            .unwrap_or(0);
        let out_path = base_dir.join(format!("recording-{ts}.wav"));

        Ok(Self {
            stream,
            state,
            sample_rate: stream_cfg.sample_rate.0,
            channels: stream_cfg.channels,
            out_path,
        })
    }

    pub fn stop(self) -> Result<PathBuf> {
        drop(self.stream);
        let state = self
            .state
            .lock()
            .map_err(|err| anyhow::anyhow!("audio sample lock error: {err}"))?;

        let state = state;
        if state.sample_count == 0 {
            log::warn!("recording has no samples; verify microphone permissions and selected input device");
        } else if state.non_zero_samples == 0 {
            log::warn!("recording contains no non-zero samples; check mic permission/device");
            if cfg!(target_os = "macos") {
                log::warn!(
                    "if running from terminal, microphone permission may be denied for this binary; package/run via a Tauri app bundle for permission prompt."
                );
            }
        }

        let raw_peak = state.max_abs;
        let mut normalize_gain = 1.0f32;
        if raw_peak > NOISY_EPS {
            normalize_gain = (TARGET_PEAK / raw_peak).min(MAX_NORMALIZE_GAIN);
            log::debug!("recording normalization gain {normalize_gain:.4} (raw_peak={raw_peak:.8})");
        } else {
            log::warn!("raw audio peak is too low for normalization; writing unmodified silent stream");
        }

        let spec = hound::WavSpec {
            channels: self.channels,
            sample_rate: self.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&self.out_path, spec)?;
        for sample in state.samples.iter() {
            let sample = sample * normalize_gain;
            writer.write_sample(quantize_i16(sample))?;
        }
        writer.finalize()?;

        log::info!(
            "recording completed: samples={} non_zero={} raw_peak={:.8} sample_rate={} channels={}",
            state.sample_count,
            state.non_zero_samples,
            state.max_abs,
            self.sample_rate,
            self.channels
        );

        Ok(self.out_path)
    }
}

#[cfg(target_os = "macos")]
fn report_runtime_context() {
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => {
            log::warn!("failed to resolve current executable path: {err}");
            return;
        }
    };

    let exe_path = exe.to_string_lossy();
    if exe_path.contains(".app/Contents/MacOS/") {
        log::info!("macOS runtime: running from app bundle: {exe_path}");
    } else {
        log::warn!(
            "macOS runtime: running from non-bundle executable: {exe_path}. Microphone permission prompts are often tied to the app bundle."
        );
        log::warn!(
            "Run with a built `.app` when validating microphone permissions, or test directly under `src-tauri/target/release/bundle/macos`."
        );
        log::warn!(
            "Clipboard usage (`Ctrl+V` / arboard) does not trigger a macOS permission prompt in this app context."
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn report_runtime_context() {}

#[derive(Debug, Default)]
struct RecorderState {
    samples: Vec<f32>,
    sample_count: usize,
    non_zero_samples: usize,
    max_abs: f32,
}

#[inline]
fn to_f32(sample: i16) -> f32 {
    sample as f32 / i16::MAX as f32
}

fn log_available_input_devices(host: &cpal::Host) {
    let devices = match host.input_devices() {
        Ok(devices) => devices,
        Err(err) => {
            log::warn!("unable to enumerate input devices: {err}");
            return;
        }
    };

    let mut had_device = false;
    for (idx, device) in devices.enumerate() {
        let name = match device.name() {
            Ok(name) => name,
            Err(_) => "unknown".to_string(),
        };
        log::info!("available input device [{idx}]: {name}");
        had_device = true;
    }

    if !had_device {
        log::warn!("no input devices found when enumerating");
    }
}

fn select_input_device(
    host: &cpal::Host,
    configured_name: Option<&str>,
) -> Result<cpal::Device> {
    log_available_input_devices(host);

    if let Some(name_hint) = configured_name {
        let name_hint = name_hint.trim().to_lowercase();
        if !name_hint.is_empty() {
            let devices = host
                .input_devices()
                .context("unable to enumerate input devices for configured input_device")?;
            for device in devices {
                let Ok(name) = device.name() else {
                    continue;
                };
                if name.to_lowercase().contains(&name_hint) {
                    log::info!("using configured input device: {name}");
                    log_supported_configs(&device);
                    return Ok(device);
                }
            }
            log::warn!(
                "configured audio input device '{name_hint}' not found; falling back to default input"
            );
        }
    }

    let device = host
        .default_input_device()
        .context("no default input device found")?;
    log_supported_configs(&device);
    Ok(device)
}

fn log_supported_configs(device: &cpal::Device) {
    let Ok(configs) = device.supported_input_configs() else {
        log::warn!("unable to read supported input configs for selected device");
        return;
    };

    for config in configs {
        let channels = config.channels();
        log::info!(
            "supported input config: format={:?} channels={} sample_rate=[{}..={}]",
            config.sample_format(),
            channels,
            config.min_sample_rate().0,
            config.max_sample_rate().0
        );
    }
}

#[inline]
fn to_f32_u16(sample: u16) -> f32 {
    sample as f32 / u16::MAX as f32 - 0.5
}

#[inline]
fn quantize_i16(sample: f32) -> i16 {
    (sample * i16::MAX as f32)
        .clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

fn push_sample(state: &mut RecorderState, sample: f32) {
    state.samples.push(sample);
    state.sample_count += 1;
    if sample.abs() > 1e-8 {
        state.non_zero_samples += 1;
        state.max_abs = state.max_abs.max(sample.abs());
    }
}

fn build_input_stream_for_format(
    device: &cpal::Device,
    sample_format: SampleFormat,
    stream_cfg: &StreamConfig,
    state: Arc<Mutex<RecorderState>>,
) -> Result<Stream> {
    match sample_format {
        SampleFormat::F32 => {
            let state = Arc::clone(&state);
            Ok(device.build_input_stream(
                stream_cfg,
                move |data: &[f32], _| {
                    if let Ok(mut state) = state.lock() {
                        for sample in data.iter() {
                            push_sample(&mut state, *sample);
                        }
                    }
                },
                |err| log::warn!("audio stream error: {err}"),
                None,
            )?)
        }
        SampleFormat::I16 => {
            let state = Arc::clone(&state);
            Ok(device.build_input_stream(
                stream_cfg,
                move |data: &[i16], _| {
                    if let Ok(mut state) = state.lock() {
                        for sample in data.iter() {
                            push_sample(&mut state, to_f32(*sample));
                        }
                    }
                },
                |err| log::warn!("audio stream error: {err}"),
                None,
            )?)
        }
        SampleFormat::U16 => {
            let state = Arc::clone(&state);
            Ok(device.build_input_stream(
                stream_cfg,
                move |data: &[u16], _| {
                    if let Ok(mut state) = state.lock() {
                        for sample in data.iter() {
                            let centered = to_f32_u16(*sample) * 2.0;
                            push_sample(&mut state, centered);
                        }
                    }
                },
                |err| log::warn!("audio stream error: {err}"),
                None,
            )?)
        }
        _ => Err(anyhow::anyhow!("unsupported sample format")),
    }
}
