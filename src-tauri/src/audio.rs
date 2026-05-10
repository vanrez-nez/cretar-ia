use crate::config::AudioCaptureConfig;
use crate::contracts::audio_level::{AudioLevelSample, AudioLevelSender, AUDIO_SPECTRUM_BANDS};
use crate::contracts::errors::RecordingErrorCode;
use crate::contracts::events::RecordingArtifact;
use crate::contracts::events::RecordingEvent;
use crate::recording::command_bus::CommandBusTx;
use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleFormat, SampleRate, Stream, StreamConfig};
use std::f32::consts::PI;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TARGET_PEAK: f32 = 0.2;
const MAX_NORMALIZE_GAIN: f32 = 64.0;
const NOISY_EPS: f32 = 1e-7;
const DEFAULT_CAPTURE_BUFFER_SAMPLES: usize = 1_048_576;
const MIN_CAPTURE_BUFFER_SAMPLES: usize = 4_096;
const MAX_CAPTURE_BUFFER_SAMPLES: usize = 4_194_304;
const CAPTURE_BUFFER_ENV: &str = "CRETAR_IA_RECORDING_BUFFER_SAMPLES";
const AUDIO_LEVEL_EMIT_INTERVAL: Duration = Duration::from_millis(50);
const AUDIO_SPECTRUM_FFT_SIZE: usize = 512;
const AUDIO_SPECTRUM_MIN_HZ: f32 = 85.0;
const AUDIO_SPECTRUM_MAX_HZ: f32 = 5_000.0;
const AUDIO_SPECTRUM_VISUAL_GAIN: f32 = 24.0;

pub(crate) fn available_input_device_names() -> Vec<String> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for device in devices {
        let Ok(name) = device.name() else {
            continue;
        };
        let Ok(mut supported_configs) = device.supported_input_configs() else {
            log::debug!("skipping input device '{name}' because supported configs are unavailable");
            continue;
        };
        if supported_configs.next().is_none() {
            log::debug!(
                "skipping input device '{name}' because it exposes no supported input configs"
            );
            continue;
        }
        if !names.iter().any(|existing| existing == &name) {
            names.push(name);
        }
    }
    names
}

pub(crate) fn effective_input_device_name(
    configured_name: Option<&str>,
    auto_switch: bool,
) -> Option<String> {
    let host = cpal::default_host();

    if let Some(name_hint) = configured_name.and_then(normalized_device_name) {
        if let Some(name) = exact_input_device_name(&host, &name_hint) {
            return Some(name);
        }
        if !auto_switch {
            return None;
        }
        log::warn!(
            "configured audio input device '{name_hint}' not found for tray selection; showing default input"
        );
    }

    default_input_device_name(&host)
}

pub(crate) fn input_device_ready(config: &AudioCaptureConfig) -> bool {
    effective_input_device_name(
        config.input_device.as_deref(),
        config.auto_switch_to_primary_device,
    )
    .is_some()
}

pub struct Recorder {
    stream: Option<Stream>,
    state: Arc<Mutex<RecorderState>>,
    stop_requested: Arc<AtomicBool>,
    callback_drained: Arc<AtomicBool>,
    sample_rate: u32,
    channels: u16,
    out_path: PathBuf,
}

#[derive(Debug, Default)]
struct AudioStreamStopProfile {
    callback_drained: bool,
    drain_ms: u128,
    pause_drop_ms: u128,
    stream_present: bool,
}

impl Recorder {
    pub fn start(
        config: &AudioCaptureConfig,
        base_dir: PathBuf,
        event_tx: CommandBusTx,
        audio_level_tx: Option<AudioLevelSender>,
    ) -> Result<Self> {
        report_runtime_context();
        let host = cpal::default_host();
        let device = select_input_device(&host, config)?;
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

        std::fs::create_dir_all(&base_dir).context("creating recording directory")?;
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|v| v.as_millis())
            .unwrap_or(0);
        let out_path = base_dir.join(format!("recording-{ts}.wav"));
        let spool_path = base_dir.join(format!("recording-{ts}.raw-f32.tmp"));
        let checkpoint_samples = capture_buffer_sample_limit();

        log::debug!(
            "audio capture buffer limit: {checkpoint_samples} samples before persistence checkpoint"
        );

        let state = Arc::new(Mutex::new(RecorderState::new(
            spool_path.clone(),
            checkpoint_samples,
        )?));

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
        let stream_error_reported = Arc::new(AtomicBool::new(false));
        let stop_requested = Arc::new(AtomicBool::new(false));
        let callback_drained = Arc::new(AtomicBool::new(false));

        let (stream_cfg, stream) = match build_input_stream_for_format(
            &device,
            supported.sample_format(),
            &requested,
            Arc::clone(&state),
            event_tx.clone(),
            audio_level_tx.clone(),
            requested.sample_rate.0,
            requested.channels,
            Arc::clone(&stream_error_reported),
            Arc::clone(&stop_requested),
            Arc::clone(&callback_drained),
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
                        event_tx.clone(),
                        audio_level_tx.clone(),
                        fallback.sample_rate.0,
                        fallback.channels,
                        Arc::clone(&stream_error_reported),
                        Arc::clone(&stop_requested),
                        Arc::clone(&callback_drained),
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

        Ok(Self {
            stream: Some(stream),
            state,
            stop_requested,
            callback_drained,
            sample_rate: stream_cfg.sample_rate.0,
            channels: stream_cfg.channels,
            out_path,
        })
    }

    pub fn stop(mut self) -> Result<RecordingArtifact> {
        let stop_started_at = Instant::now();
        let stream_profile = self.stop_audio_stream();
        let lock_started_at = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|err| anyhow::anyhow!("audio sample lock error: {err}"))?;
        let state_lock_ms = lock_started_at.elapsed().as_millis();

        if let Some(err) = state.flush_error.take() {
            state.close_spool()?;
            let _ = std::fs::remove_file(&state.spool_path);
            return Err(anyhow::anyhow!(
                "audio capture persistence failed before stop; memory was bounded by dropping buffered samples: {err}"
            ));
        }

        let flush_started_at = Instant::now();
        state
            .flush_buffer()
            .context("flushing final audio capture buffer")?;
        state.close_spool().context("closing audio capture spool")?;
        let flush_close_ms = flush_started_at.elapsed().as_millis();

        if state.sample_count == 0 {
            log::warn!(
                "recording has no samples; verify microphone permissions and selected input device"
            );
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
            log::debug!(
                "recording normalization gain {normalize_gain:.4} (raw_peak={raw_peak:.8})"
            );
        } else {
            log::warn!(
                "raw audio peak is too low for normalization; writing unmodified silent stream"
            );
        }

        let spec = hound::WavSpec {
            channels: self.channels,
            sample_rate: self.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let wav_write_started_at = Instant::now();
        write_spooled_wav(
            &state.spool_path,
            &self.out_path,
            spec,
            normalize_gain,
            state.sample_count,
        )?;
        let wav_write_ms = wav_write_started_at.elapsed().as_millis();
        let wav_bytes = std::fs::metadata(&self.out_path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let spool_cleanup_started_at = Instant::now();
        let spool_cleanup_success = std::fs::remove_file(&state.spool_path).is_ok();
        let spool_cleanup_ms = spool_cleanup_started_at.elapsed().as_millis();

        log::info!(
            "recording completed: samples={} non_zero={} raw_peak={:.8} sample_rate={} channels={} checkpoints={}",
            state.sample_count,
            state.non_zero_samples,
            state.max_abs,
            self.sample_rate,
            self.channels,
            state.persistence_checkpoints
        );

        log::info!(
            "profile.audio_finalize success=true total_stop_to_wav_ready_ms={} stream_drain_ms={} stream_pause_drop_ms={} callback_drained={} stream_present={} state_lock_ms={} flush_close_ms={} wav_write_ms={} spool_cleanup_ms={} spool_cleanup_success={} samples={} non_zero={} raw_peak={:.8} sample_rate={} channels={} checkpoints={} wav_bytes={} wav_path={}",
            stop_started_at.elapsed().as_millis(),
            stream_profile.drain_ms,
            stream_profile.pause_drop_ms,
            stream_profile.callback_drained,
            stream_profile.stream_present,
            state_lock_ms,
            flush_close_ms,
            wav_write_ms,
            spool_cleanup_ms,
            spool_cleanup_success,
            state.sample_count,
            state.non_zero_samples,
            state.max_abs,
            self.sample_rate,
            self.channels,
            state.persistence_checkpoints,
            wav_bytes,
            self.out_path.display()
        );

        Ok(RecordingArtifact {
            path: self.out_path.clone(),
            duration_ms: recording_duration_ms(state.sample_count, self.sample_rate, self.channels),
        })
    }

    fn stop_audio_stream(&mut self) -> AudioStreamStopProfile {
        self.stop_requested.store(true, Ordering::SeqCst);

        let drain_start = Instant::now();
        while !self.callback_drained.load(Ordering::SeqCst)
            && drain_start.elapsed() < Duration::from_millis(200)
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        let drain_ms = drain_start.elapsed().as_millis();
        let callback_drained = self.callback_drained.load(Ordering::SeqCst);

        if callback_drained {
            log::info!("audio callback drain completed before stream shutdown");
        } else {
            log::warn!(
                "audio callback drain timed out after {}ms; pausing stream anyway",
                drain_ms
            );
        }

        let Some(stream) = self.stream.take() else {
            return AudioStreamStopProfile {
                callback_drained,
                drain_ms,
                pause_drop_ms: 0,
                stream_present: false,
            };
        };

        let pause_drop_started_at = Instant::now();
        if let Err(err) = stream.pause() {
            log::warn!("failed to pause audio input stream before drop: {err}");
        }
        drop(stream);
        let pause_drop_ms = pause_drop_started_at.elapsed().as_millis();
        log::info!("audio input stream paused and dropped");
        AudioStreamStopProfile {
            callback_drained,
            drain_ms,
            pause_drop_ms,
            stream_present: true,
        }
    }
}

fn recording_duration_ms(sample_count: usize, sample_rate: u32, channels: u16) -> u64 {
    let frames_per_second = u64::from(sample_rate).saturating_mul(u64::from(channels));
    if frames_per_second == 0 {
        return 0;
    }
    (sample_count as u64).saturating_mul(1_000) / frames_per_second
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.stream.is_some() {
            let _ = self.stop_audio_stream();
        }
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

struct RecorderState {
    buffer: Vec<f32>,
    sample_count: usize,
    non_zero_samples: usize,
    max_abs: f32,
    checkpoint_samples: usize,
    persistence_checkpoints: usize,
    spool_path: PathBuf,
    spool_writer: Option<BufWriter<File>>,
    flush_error: Option<String>,
}

impl RecorderState {
    fn new(spool_path: PathBuf, checkpoint_samples: usize) -> Result<Self> {
        let spool = File::create(&spool_path)
            .with_context(|| format!("creating audio capture spool {:?}", spool_path))?;
        Ok(Self {
            buffer: Vec::with_capacity(checkpoint_samples.min(65_536)),
            sample_count: 0,
            non_zero_samples: 0,
            max_abs: 0.0,
            checkpoint_samples,
            persistence_checkpoints: 0,
            spool_path,
            spool_writer: Some(BufWriter::new(spool)),
            flush_error: None,
        })
    }

    fn flush_buffer(&mut self) -> Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let Some(writer) = self.spool_writer.as_mut() else {
            return Err(anyhow::anyhow!("audio capture spool is already closed"));
        };

        for sample in self.buffer.drain(..) {
            writer.write_all(&sample.to_le_bytes())?;
        }
        writer.flush()?;
        self.persistence_checkpoints += 1;
        Ok(())
    }

    fn close_spool(&mut self) -> Result<()> {
        if let Some(mut writer) = self.spool_writer.take() {
            writer.flush()?;
        }
        Ok(())
    }
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

fn select_input_device(host: &cpal::Host, config: &AudioCaptureConfig) -> Result<cpal::Device> {
    log_available_input_devices(host);

    if let Some(name_hint) = config.input_device.as_deref() {
        if let Some(name_hint) = normalized_device_name(name_hint) {
            let devices = host
                .input_devices()
                .context("unable to enumerate input devices for configured input_device")?;
            for device in devices {
                let Ok(name) = device.name() else {
                    continue;
                };
                if device_names_match(&name, &name_hint) {
                    log::info!("using configured input device: {name}");
                    log_supported_configs(&device);
                    return Ok(device);
                }
            }
            if !config.auto_switch_to_primary_device {
                return Err(anyhow::anyhow!(
                    "configured audio input device '{name_hint}' is not available and audio.auto_switch_to_primary_device is disabled"
                ));
            }
            log::warn!(
                "configured audio input device '{name_hint}' not found; falling back to default input because audio.auto_switch_to_primary_device is enabled"
            );
        }
    }

    let device = host
        .default_input_device()
        .context("no default input device found")?;
    log_supported_configs(&device);
    Ok(device)
}

fn exact_input_device_name(host: &cpal::Host, name_hint: &str) -> Option<String> {
    let devices = host.input_devices().ok()?;
    for device in devices {
        let Ok(name) = device.name() else {
            continue;
        };
        if device_names_match(&name, name_hint) {
            return Some(name);
        }
    }
    None
}

fn default_input_device_name(host: &cpal::Host) -> Option<String> {
    host.default_input_device()
        .and_then(|device| device.name().ok())
}

fn normalized_device_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn device_names_match(actual: &str, expected: &str) -> bool {
    actual.trim().eq_ignore_ascii_case(expected.trim())
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
    (sample * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

#[derive(Clone, Copy, Debug, Default)]
struct ComplexSample {
    re: f32,
    im: f32,
}

impl ComplexSample {
    const fn zero() -> Self {
        Self { re: 0.0, im: 0.0 }
    }

    fn from_polar(radius: f32, angle: f32) -> Self {
        Self {
            re: radius * angle.cos(),
            im: radius * angle.sin(),
        }
    }

    fn magnitude(self) -> f32 {
        (self.re * self.re + self.im * self.im).sqrt()
    }
}

impl std::ops::Add for ComplexSample {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            re: self.re + rhs.re,
            im: self.im + rhs.im,
        }
    }
}

impl std::ops::Sub for ComplexSample {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            re: self.re - rhs.re,
            im: self.im - rhs.im,
        }
    }
}

impl std::ops::Mul for ComplexSample {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self {
            re: self.re * rhs.re - self.im * rhs.im,
            im: self.re * rhs.im + self.im * rhs.re,
        }
    }
}

struct AudioLevelTelemetry {
    tx: AudioLevelSender,
    last_emit: Instant,
    sample_rate: f32,
    channel_count: u16,
    channel_cursor: u16,
    samples: [f32; AUDIO_SPECTRUM_FFT_SIZE],
    next_sample_index: usize,
    samples_ready: usize,
    fft_buffer: [ComplexSample; AUDIO_SPECTRUM_FFT_SIZE],
}

impl AudioLevelTelemetry {
    fn new(tx: AudioLevelSender, sample_rate: u32, channels: u16) -> Self {
        let now = Instant::now();
        Self {
            tx,
            last_emit: now.checked_sub(AUDIO_LEVEL_EMIT_INTERVAL).unwrap_or(now),
            sample_rate: sample_rate as f32,
            channel_count: channels.max(1),
            channel_cursor: 0,
            samples: [0.0; AUDIO_SPECTRUM_FFT_SIZE],
            next_sample_index: 0,
            samples_ready: 0,
            fft_buffer: [ComplexSample::zero(); AUDIO_SPECTRUM_FFT_SIZE],
        }
    }

    fn push_sample(&mut self, sample: f32) {
        if self.channel_cursor == 0 {
            self.samples[self.next_sample_index] = sample;
            self.next_sample_index = (self.next_sample_index + 1) % AUDIO_SPECTRUM_FFT_SIZE;
            self.samples_ready = self
                .samples_ready
                .saturating_add(1)
                .min(AUDIO_SPECTRUM_FFT_SIZE);
        }
        self.channel_cursor = (self.channel_cursor + 1) % self.channel_count;
    }

    fn maybe_emit(&mut self) {
        if self.samples_ready < AUDIO_SPECTRUM_FFT_SIZE
            || self.last_emit.elapsed() < AUDIO_LEVEL_EMIT_INTERVAL
        {
            return;
        }

        self.last_emit = Instant::now();
        let levels = self.spectrum_levels();
        let _ = self.tx.try_send(AudioLevelSample { levels });
    }

    fn spectrum_levels(&mut self) -> [f32; AUDIO_SPECTRUM_BANDS] {
        let oldest_sample = self.next_sample_index;
        for idx in 0..AUDIO_SPECTRUM_FFT_SIZE {
            let sample = self.samples[(oldest_sample + idx) % AUDIO_SPECTRUM_FFT_SIZE];
            let window = hann_window(idx, AUDIO_SPECTRUM_FFT_SIZE);
            self.fft_buffer[idx] = ComplexSample {
                re: sample * window,
                im: 0.0,
            };
        }

        fft_in_place(&mut self.fft_buffer);
        spectrum_bands(&self.fft_buffer, self.sample_rate)
    }
}

fn hann_window(index: usize, size: usize) -> f32 {
    if size <= 1 {
        return 1.0;
    }
    0.5 - 0.5 * ((2.0 * PI * index as f32) / (size - 1) as f32).cos()
}

fn fft_in_place(buffer: &mut [ComplexSample; AUDIO_SPECTRUM_FFT_SIZE]) {
    let mut reversed = 0usize;
    for index in 1..AUDIO_SPECTRUM_FFT_SIZE {
        let mut bit = AUDIO_SPECTRUM_FFT_SIZE >> 1;
        while reversed & bit != 0 {
            reversed ^= bit;
            bit >>= 1;
        }
        reversed ^= bit;
        if index < reversed {
            buffer.swap(index, reversed);
        }
    }

    let mut len = 2usize;
    while len <= AUDIO_SPECTRUM_FFT_SIZE {
        let angle = -2.0 * PI / len as f32;
        let w_len = ComplexSample::from_polar(1.0, angle);
        let half = len / 2;
        let mut start = 0usize;
        while start < AUDIO_SPECTRUM_FFT_SIZE {
            let mut w = ComplexSample { re: 1.0, im: 0.0 };
            for idx in 0..half {
                let even = buffer[start + idx];
                let odd = buffer[start + idx + half] * w;
                buffer[start + idx] = even + odd;
                buffer[start + idx + half] = even - odd;
                w = w * w_len;
            }
            start += len;
        }
        len *= 2;
    }
}

fn spectrum_bands(
    fft_buffer: &[ComplexSample; AUDIO_SPECTRUM_FFT_SIZE],
    sample_rate: f32,
) -> [f32; AUDIO_SPECTRUM_BANDS] {
    let nyquist = sample_rate * 0.5;
    let max_hz = AUDIO_SPECTRUM_MAX_HZ.min(nyquist * 0.92);
    let min_hz = AUDIO_SPECTRUM_MIN_HZ.min(max_hz * 0.5).max(1.0);
    let log_min = min_hz.ln();
    let log_max = max_hz.max(min_hz + 1.0).ln();
    let hz_per_bin = sample_rate / AUDIO_SPECTRUM_FFT_SIZE as f32;
    let mut levels = [0.0; AUDIO_SPECTRUM_BANDS];

    for (band, level) in levels.iter_mut().enumerate() {
        let band_start = band as f32 / AUDIO_SPECTRUM_BANDS as f32;
        let band_end = (band + 1) as f32 / AUDIO_SPECTRUM_BANDS as f32;
        let start_hz = (log_min + (log_max - log_min) * band_start).exp();
        let end_hz = (log_min + (log_max - log_min) * band_end).exp();
        let start_bin = ((start_hz / hz_per_bin).floor() as usize)
            .max(1)
            .min((AUDIO_SPECTRUM_FFT_SIZE / 2) - 1);
        let end_bin = ((end_hz / hz_per_bin).ceil() as usize)
            .max(start_bin + 1)
            .min(AUDIO_SPECTRUM_FFT_SIZE / 2);

        let mut sum = 0.0;
        let mut count = 0usize;
        for bin in start_bin..end_bin {
            sum += fft_buffer[bin].magnitude() / AUDIO_SPECTRUM_FFT_SIZE as f32;
            count += 1;
        }

        let average = if count > 0 { sum / count as f32 } else { 0.0 };
        let visual_level = (average * AUDIO_SPECTRUM_VISUAL_GAIN).sqrt();
        *level = if visual_level.is_finite() {
            visual_level.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    levels
}

fn push_sample(state: &mut RecorderState, sample: f32) {
    if state.flush_error.is_some() {
        return;
    }

    if state.spool_writer.is_none() {
        return;
    }

    state.buffer.push(sample);
    state.sample_count += 1;
    if sample.abs() > 1e-8 {
        state.non_zero_samples += 1;
        state.max_abs = state.max_abs.max(sample.abs());
    }

    if state.buffer.len() >= state.checkpoint_samples {
        if let Err(err) = state.flush_buffer() {
            state.flush_error = Some(err.to_string());
            state.buffer.clear();
            log::warn!(
                "audio capture persistence failed; dropping subsequent samples to keep memory bounded: {err}"
            );
        }
    }
}

fn capture_buffer_sample_limit() -> usize {
    std::env::var(CAPTURE_BUFFER_ENV)
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(DEFAULT_CAPTURE_BUFFER_SAMPLES)
        .clamp(MIN_CAPTURE_BUFFER_SAMPLES, MAX_CAPTURE_BUFFER_SAMPLES)
}

fn write_spooled_wav(
    spool_path: &PathBuf,
    out_path: &PathBuf,
    spec: hound::WavSpec,
    normalize_gain: f32,
    sample_count: usize,
) -> Result<()> {
    let mut reader = BufReader::new(
        File::open(spool_path)
            .with_context(|| format!("opening audio capture spool {:?}", spool_path))?,
    );
    let mut writer = hound::WavWriter::create(out_path, spec)?;
    let mut bytes = [0u8; 4];

    for _ in 0..sample_count {
        reader
            .read_exact(&mut bytes)
            .context("reading audio capture sample from spool")?;
        let sample = f32::from_le_bytes(bytes) * normalize_gain;
        writer.write_sample(quantize_i16(sample))?;
    }

    writer.finalize()?;
    Ok(())
}

fn build_input_stream_for_format(
    device: &cpal::Device,
    sample_format: SampleFormat,
    stream_cfg: &StreamConfig,
    state: Arc<Mutex<RecorderState>>,
    event_tx: CommandBusTx,
    audio_level_tx: Option<AudioLevelSender>,
    sample_rate: u32,
    channels: u16,
    stream_error_reported: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
    callback_drained: Arc<AtomicBool>,
) -> Result<Stream> {
    match sample_format {
        SampleFormat::F32 => {
            let state = Arc::clone(&state);
            let event_tx = event_tx.clone();
            let stream_error_reported = Arc::clone(&stream_error_reported);
            let stop_requested = Arc::clone(&stop_requested);
            let callback_drained = Arc::clone(&callback_drained);
            let mut audio_level = audio_level_tx
                .clone()
                .map(|tx| AudioLevelTelemetry::new(tx, sample_rate, channels));
            Ok(device.build_input_stream(
                stream_cfg,
                move |data: &[f32], _| {
                    if stop_requested.load(Ordering::SeqCst) {
                        callback_drained.store(true, Ordering::SeqCst);
                        return;
                    }
                    if let Ok(mut state) = state.lock() {
                        for sample in data.iter() {
                            push_sample(&mut state, *sample);
                        }
                    }
                    if let Some(audio_level) = audio_level.as_mut() {
                        for sample in data.iter() {
                            audio_level.push_sample(*sample);
                        }
                        audio_level.maybe_emit();
                    }
                },
                move |err| report_audio_stream_error(err, &event_tx, &stream_error_reported),
                None,
            )?)
        }
        SampleFormat::I16 => {
            let state = Arc::clone(&state);
            let event_tx = event_tx.clone();
            let stream_error_reported = Arc::clone(&stream_error_reported);
            let stop_requested = Arc::clone(&stop_requested);
            let callback_drained = Arc::clone(&callback_drained);
            let mut audio_level = audio_level_tx
                .clone()
                .map(|tx| AudioLevelTelemetry::new(tx, sample_rate, channels));
            Ok(device.build_input_stream(
                stream_cfg,
                move |data: &[i16], _| {
                    if stop_requested.load(Ordering::SeqCst) {
                        callback_drained.store(true, Ordering::SeqCst);
                        return;
                    }
                    if let Ok(mut state) = state.lock() {
                        for sample in data.iter() {
                            push_sample(&mut state, to_f32(*sample));
                        }
                    }
                    if let Some(audio_level) = audio_level.as_mut() {
                        for sample in data.iter() {
                            audio_level.push_sample(to_f32(*sample));
                        }
                        audio_level.maybe_emit();
                    }
                },
                move |err| report_audio_stream_error(err, &event_tx, &stream_error_reported),
                None,
            )?)
        }
        SampleFormat::U16 => {
            let state = Arc::clone(&state);
            let event_tx = event_tx.clone();
            let stream_error_reported = Arc::clone(&stream_error_reported);
            let stop_requested = Arc::clone(&stop_requested);
            let callback_drained = Arc::clone(&callback_drained);
            let mut audio_level = audio_level_tx
                .clone()
                .map(|tx| AudioLevelTelemetry::new(tx, sample_rate, channels));
            Ok(device.build_input_stream(
                stream_cfg,
                move |data: &[u16], _| {
                    if stop_requested.load(Ordering::SeqCst) {
                        callback_drained.store(true, Ordering::SeqCst);
                        return;
                    }
                    if let Ok(mut state) = state.lock() {
                        for sample in data.iter() {
                            let centered = to_f32_u16(*sample) * 2.0;
                            push_sample(&mut state, centered);
                        }
                    }
                    if let Some(audio_level) = audio_level.as_mut() {
                        for sample in data.iter() {
                            audio_level.push_sample(to_f32_u16(*sample) * 2.0);
                        }
                        audio_level.maybe_emit();
                    }
                },
                move |err| report_audio_stream_error(err, &event_tx, &stream_error_reported),
                None,
            )?)
        }
        _ => Err(anyhow::anyhow!("unsupported sample format")),
    }
}

fn report_audio_stream_error(
    err: cpal::StreamError,
    event_tx: &CommandBusTx,
    stream_error_reported: &AtomicBool,
) {
    log::warn!("audio stream error: {err}");
    if stream_error_reported.swap(true, Ordering::Relaxed) {
        return;
    }

    let event = RecordingEvent::AudioDeviceUnavailable {
        code: RecordingErrorCode::AudioInit,
        reason: format!("audio stream error: {err}"),
    };
    if event_tx.send_worker(event).is_some() {
        log::warn!("audio device unavailable event dropped because worker queue was full");
    }
}
