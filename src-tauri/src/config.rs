use crate::contracts::events::PipelineMode;
use crate::hotkey::validate_shortcut;
use anyhow::{anyhow, Context, Result};
use dirs::home_dir;
use serde::{Deserialize, Serialize};
use serde_json;
use std::fs;
use std::path::{Path, PathBuf};

mod migration;

const DEFAULT_DEBOUNCE_MS: u64 = 120;
const DEFAULT_HOTKEY_QUEUE_CAPACITY: u32 = 64;
const DEFAULT_WORKER_QUEUE_CAPACITY: u32 = 128;
const DEFAULT_SETTLE_TIMEOUT_MS: u64 = 800;
const DEFAULT_MAX_RECORDING_DURATION_SECS: u64 = 0;
const DEFAULT_OPENROUTER_MAX_AUDIO_BYTES: u64 = 24 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionMode {
    PushToTalk,
    Toggle,
}

impl From<PipelineMode> for InteractionMode {
    fn from(mode: PipelineMode) -> Self {
        match mode {
            PipelineMode::PushToTalk => InteractionMode::PushToTalk,
            PipelineMode::Toggle => InteractionMode::Toggle,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    ClipboardOnly,
    ClipboardPaste,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueueSaturationPolicy {
    #[default]
    Retry,
    ErrorOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RecoveryStrategyConfig {
    #[serde(default = "RecoveryStrategyConfig::default_retry_start_timeout")]
    pub retry_start_timeout: bool,
    #[serde(default = "RecoveryStrategyConfig::default_retry_stop_timeout")]
    pub retry_stop_timeout: bool,
    #[serde(default = "RecoveryStrategyConfig::default_retry_processing_timeout")]
    pub retry_processing_timeout: bool,
    #[serde(default = "RecoveryStrategyConfig::default_retry_queue_saturation")]
    pub retry_queue_saturation: bool,
}

impl RecoveryStrategyConfig {
    fn default_retry_start_timeout() -> bool {
        true
    }

    fn default_retry_stop_timeout() -> bool {
        true
    }

    fn default_retry_processing_timeout() -> bool {
        true
    }

    fn default_retry_queue_saturation() -> bool {
        true
    }
}

impl Default for RecoveryStrategyConfig {
    fn default() -> Self {
        Self {
            retry_start_timeout: Self::default_retry_start_timeout(),
            retry_stop_timeout: Self::default_retry_stop_timeout(),
            retry_processing_timeout: Self::default_retry_processing_timeout(),
            retry_queue_saturation: Self::default_retry_queue_saturation(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PipelineConfig {
    #[serde(default)]
    pub debounce_ms: Option<u64>,
    #[serde(default)]
    pub settle_timeout_ms: Option<u64>,
    #[serde(default)]
    pub hotkey_queue_capacity: Option<u32>,
    #[serde(default)]
    pub worker_queue_capacity: Option<u32>,
    #[serde(default)]
    pub queue_saturation_policy: Option<QueueSaturationPolicy>,
    #[serde(default)]
    pub recovery: Option<RecoveryStrategyConfig>,
    #[serde(default)]
    pub max_recording_duration_secs: Option<u64>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            debounce_ms: None,
            settle_timeout_ms: None,
            hotkey_queue_capacity: None,
            worker_queue_capacity: None,
            queue_saturation_policy: None,
            recovery: None,
            max_recording_duration_secs: None,
        }
    }
}

impl PipelineConfig {
    fn effective_debounce_ms(&self, legacy: u64) -> u64 {
        self.debounce_ms
            .filter(|value| *value > 0)
            .or_else(|| if legacy > 0 { Some(legacy) } else { None })
            .unwrap_or(DEFAULT_DEBOUNCE_MS)
    }

    fn effective_settle_timeout_ms(&self, legacy: u64) -> u64 {
        self.settle_timeout_ms
            .filter(|value| *value > 0)
            .or_else(|| if legacy > 0 { Some(legacy) } else { None })
            .unwrap_or(DEFAULT_SETTLE_TIMEOUT_MS)
    }

    fn effective_hotkey_queue_capacity(&self, legacy: u32) -> u32 {
        self.hotkey_queue_capacity
            .filter(|value| *value > 0)
            .or_else(|| if legacy > 0 { Some(legacy) } else { None })
            .unwrap_or(DEFAULT_HOTKEY_QUEUE_CAPACITY)
    }

    fn effective_worker_queue_capacity(&self, legacy: u32) -> u32 {
        self.worker_queue_capacity
            .filter(|value| *value > 0)
            .or_else(|| if legacy > 0 { Some(legacy) } else { None })
            .unwrap_or(DEFAULT_WORKER_QUEUE_CAPACITY)
    }

    fn queue_saturation_policy(&self) -> QueueSaturationPolicy {
        self.queue_saturation_policy.unwrap_or_default()
    }

    fn recovery_strategy(&self) -> RecoveryStrategyConfig {
        self.recovery.unwrap_or_default()
    }

    fn max_recording_duration_secs(&self, legacy: u64) -> Option<u64> {
        self.max_recording_duration_secs
            .filter(|value| *value > 0)
            .or_else(|| if legacy > 0 { Some(legacy) } else { None })
    }

    fn validate_with_legacy(&self, issues: &mut Vec<String>, legacy: &InteractionConfig, audio: &AudioCaptureConfig) {
        if let Some(debounce_ms) = self.debounce_ms {
            if debounce_ms == 0 {
                issues.push("pipeline.debounce_ms must be greater than 0 when set".to_string());
            } else if debounce_ms > 3_600_000 {
                issues.push("pipeline.debounce_ms must be at most 3,600,000".to_string());
            }

            if legacy.repeat_debounce_ms > 0 && legacy.repeat_debounce_ms != debounce_ms {
                issues.push("pipeline.debounce_ms and interaction.repeat_debounce_ms must match when both are set".to_string());
            }
        }

        if let Some(settle_timeout_ms) = self.settle_timeout_ms {
            if settle_timeout_ms < 100 {
                issues.push("pipeline.settle_timeout_ms must be at least 100".to_string());
            }
            if settle_timeout_ms > 120_000 {
                issues.push("pipeline.settle_timeout_ms must be at most 120000".to_string());
            }
        }

        if let Some(capacity) = self.hotkey_queue_capacity {
            if capacity == 0 {
                issues.push("pipeline.hotkey_queue_capacity must be greater than 0 when set".to_string());
            } else if capacity > 5_000 {
                issues.push("pipeline.hotkey_queue_capacity must be at most 5000".to_string());
            }

            if legacy.hotkey_queue_capacity > 0 && legacy.hotkey_queue_capacity != capacity {
                issues.push(
                    "pipeline.hotkey_queue_capacity and interaction.hotkey_queue_capacity must match when both are set".to_string(),
                );
            }
        }

        if let Some(capacity) = self.worker_queue_capacity {
            if capacity == 0 {
                issues.push("pipeline.worker_queue_capacity must be greater than 0 when set".to_string());
            } else if capacity > 20_000 {
                issues.push("pipeline.worker_queue_capacity must be at most 20,000".to_string());
            }

            if legacy.worker_queue_capacity > 0 && legacy.worker_queue_capacity != capacity {
                issues.push(
                    "pipeline.worker_queue_capacity and interaction.worker_queue_capacity must match when both are set".to_string(),
                );
            }
        }

        if let Some(max_secs) = self.max_recording_duration_secs {
            if max_secs == 0 {
                issues.push("pipeline.max_recording_duration_secs must be greater than 0 when set".to_string());
            } else if max_secs > 86_400 {
                issues.push("pipeline.max_recording_duration_secs must be at most 86,400".to_string());
            } else if audio.max_duration_secs > 0 && max_secs != audio.max_duration_secs {
                issues.push(
                    "pipeline.max_recording_duration_secs must match audio.max_duration_secs when both are set".to_string(),
                );
            }
        }

        if self.queue_saturation_policy == Some(QueueSaturationPolicy::ErrorOnly)
            && self.recovery_strategy().retry_queue_saturation
        {
            issues.push(
                "pipeline.recovery.retry_queue_saturation cannot be true when queue_saturation_policy is error_only".to_string(),
            );
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionConfig {
    pub mode: InteractionMode,
    pub shortcut: String,
    #[serde(default)]
    pub hotkey_queue_capacity: u32,
    #[serde(default)]
    pub worker_queue_capacity: u32,
    #[serde(default = "InteractionConfig::default_repeat_debounce_ms")]
    pub repeat_debounce_ms: u64,
}

impl Default for InteractionConfig {
    fn default() -> Self {
        Self {
            mode: InteractionMode::PushToTalk,
            shortcut: "ctrl+shift+space".to_string(),
            hotkey_queue_capacity: 0,
            worker_queue_capacity: 0,
            repeat_debounce_ms: Self::default_repeat_debounce_ms(),
        }
    }
}

impl InteractionConfig {
    fn default_repeat_debounce_ms() -> u64 {
        DEFAULT_DEBOUNCE_MS
    }

    pub fn set_mode(&mut self, mode: PipelineMode) {
        self.mode = InteractionMode::from(mode);
    }

    pub fn pipeline_mode(&self) -> PipelineMode {
        match self.mode {
            InteractionMode::PushToTalk => PipelineMode::PushToTalk,
            InteractionMode::Toggle => PipelineMode::Toggle,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterConfig {
    pub api_key: String,
    pub model: String,
    #[serde(default = "OpenRouterConfig::default_base_url")]
    pub base_url: String,
    #[serde(default = "OpenRouterConfig::default_endpoint")]
    pub endpoint: String,
    #[serde(default = "OpenRouterConfig::default_max_audio_bytes")]
    pub max_audio_bytes: u64,
    #[serde(default)]
    pub prompt: Option<String>,
}

impl OpenRouterConfig {
    fn default_base_url() -> String {
        "https://openrouter.ai/api/v1".to_string()
    }

    fn default_endpoint() -> String {
        "audio/transcriptions".to_string()
    }

    fn default_max_audio_bytes() -> u64 {
        DEFAULT_OPENROUTER_MAX_AUDIO_BYTES
    }
}

impl Default for OpenRouterConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "openai/whisper-1".to_string(),
            base_url: Self::default_base_url(),
            endpoint: Self::default_endpoint(),
            max_audio_bytes: Self::default_max_audio_bytes(),
            prompt: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub provider: String,
    pub openrouter: OpenRouterConfig,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            provider: "openrouter".to_string(),
            openrouter: OpenRouterConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioCaptureConfig {
    pub sample_rate: u32,
    pub channels: u16,
    #[serde(default)]
    pub input_device: Option<String>,
    #[serde(default = "AudioCaptureConfig::default_max_duration_secs")]
    pub max_duration_secs: u64,
    pub recording_dir: String,
}

impl AudioCaptureConfig {
    fn default_max_duration_secs() -> u64 {
        DEFAULT_MAX_RECORDING_DURATION_SECS
    }
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            sample_rate: 0,
            channels: 0,
            input_device: None,
            max_duration_secs: Self::default_max_duration_secs(),
            recording_dir: "recordings".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioCueConfig {
    pub enabled: bool,
    pub start_sound: Option<String>,
    pub stop_sound: Option<String>,
    pub error_sound: Option<String>,
    pub volume: f32,
}

impl Default for AudioCueConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            start_sound: Some("sounds/start.wav".to_string()),
            stop_sound: Some("sounds/stop.wav".to_string()),
            error_sound: Some("sounds/error_1.wav".to_string()),
            volume: 0.6,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    pub mode: OutputMode,
    #[serde(default = "OutputConfig::default_paste_delay")]
    pub paste_delay_ms: u64,
    #[serde(default = "OutputConfig::default_cleanup_recording_after_processing")]
    pub cleanup_recording_after_processing: bool,
    #[serde(default = "OutputConfig::default_processing_timeout_ms")]
    pub processing_timeout_ms: u64,
}

impl OutputConfig {
    fn default_paste_delay() -> u64 {
        40
    }

    fn default_cleanup_recording_after_processing() -> bool {
        false
    }

    fn default_processing_timeout_ms() -> u64 {
        30_000
    }
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            mode: OutputMode::ClipboardPaste,
            paste_delay_ms: Self::default_paste_delay(),
            cleanup_recording_after_processing: Self::default_cleanup_recording_after_processing(),
            processing_timeout_ms: Self::default_processing_timeout_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrayTooltipConfig {
    pub idle: String,
    pub recording: String,
    pub sending: String,
    pub success: String,
}

impl Default for TrayTooltipConfig {
    fn default() -> Self {
        Self {
            idle: "Cretar IA idle".to_string(),
            recording: "Cretar IA recording".to_string(),
            sending: "Cretar IA transcribing...".to_string(),
            success: "Cretar IA ready".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrayConfig {
    pub title: String,
    pub icon: String,
    #[serde(default)]
    pub tooltip: TrayTooltipConfig,
    #[serde(default = "TrayConfig::default_refresh_ms")]
    pub refresh_ms: u64,
}

impl TrayConfig {
    fn default_refresh_ms() -> u64 {
        3000
    }
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            title: "Cretar IA".to_string(),
            icon: "idle".to_string(),
            tooltip: TrayTooltipConfig::default(),
            refresh_ms: Self::default_refresh_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub provider: ProviderConfig,
    #[serde(default)]
    pub interaction: InteractionConfig,
    #[serde(default)]
    pub pipeline: PipelineConfig,
    #[serde(default)]
    pub audio: AudioCaptureConfig,
    #[serde(default)]
    pub audio_cues: AudioCueConfig,
    #[serde(default)]
    pub output: OutputConfig,
    #[serde(default)]
    pub tray: TrayConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            provider: ProviderConfig::default(),
            interaction: InteractionConfig::default(),
            pipeline: PipelineConfig::default(),
            audio: AudioCaptureConfig::default(),
            audio_cues: AudioCueConfig::default(),
            output: OutputConfig::default(),
            tray: TrayConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn home_dir() -> PathBuf {
        home_dir().unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn config_path() -> PathBuf {
        Self::home_dir().join(".cretar-ia").join("config.json")
    }

    pub fn base_dir() -> PathBuf {
        Self::home_dir().join(".cretar-ia")
    }

    pub fn base_dir_path(&self) -> PathBuf {
        Self::base_dir()
    }

    fn resolve_relative(base: &Path, rel: &str) -> PathBuf {
        let path = PathBuf::from(rel);
        if path.is_absolute() {
            path
        } else {
            base.join(path)
        }
    }

    pub fn recordings_path(&self) -> PathBuf {
        Self::resolve_relative(&self.base_dir_path(), &self.audio.recording_dir)
    }

    pub fn resolve_sound_path(&self, value: Option<&String>) -> Option<PathBuf> {
        value
            .as_deref()
            .map(|value| Self::resolve_relative(&self.base_dir_path(), value))
    }

    pub fn log_file_path() -> PathBuf {
        Self::base_dir().join("logs").join("app.log")
    }

    pub fn effective_repeat_debounce_ms(&self) -> u64 {
        self.pipeline
            .effective_debounce_ms(self.interaction.repeat_debounce_ms)
    }

    pub fn effective_settle_timeout_ms(&self) -> u64 {
        self.pipeline
            .effective_settle_timeout_ms(0)
    }

    pub fn effective_hotkey_queue_capacity(&self) -> u32 {
        self.pipeline
            .effective_hotkey_queue_capacity(self.interaction.hotkey_queue_capacity)
    }

    pub fn effective_worker_queue_capacity(&self) -> u32 {
        self.pipeline
            .effective_worker_queue_capacity(self.interaction.worker_queue_capacity)
    }

    pub fn queue_saturation_policy(&self) -> QueueSaturationPolicy {
        self.pipeline.queue_saturation_policy()
    }

    pub fn recovery_strategy(&self) -> RecoveryStrategyConfig {
        self.pipeline.recovery_strategy()
    }

    pub fn effective_max_recording_duration_secs(&self) -> Option<u64> {
        self.pipeline
            .max_recording_duration_secs(self.audio.max_duration_secs)
    }

    pub fn validate(&self) -> Result<()> {
        let mut issues: Vec<String> = Vec::new();

        if let Err(err) = validate_shortcut(&self.interaction.shortcut) {
            issues.push(format!("interaction.shortcut: {err}"));
        }

        if self.interaction.repeat_debounce_ms > 60_000 {
            issues.push("interaction.repeat_debounce_ms must be at most 60,000".to_string());
        }

        if self.interaction.hotkey_queue_capacity > 0 && self.interaction.hotkey_queue_capacity > 20_000 {
            issues.push("interaction.hotkey_queue_capacity must be at most 20,000".to_string());
        }

        if self.interaction.worker_queue_capacity > 0 && self.interaction.worker_queue_capacity > 20_000 {
            issues.push("interaction.worker_queue_capacity must be at most 20,000".to_string());
        }

        if self.pipeline
            .max_recording_duration_secs
            .is_some_and(|value| value == 0)
        {
            issues.push("pipeline.max_recording_duration_secs must be greater than 0 when set".to_string());
        }

        if self.audio.sample_rate > 0 && self.audio.sample_rate < 8_000 {
            issues.push("audio.sample_rate must be 0 or at least 8000".to_string());
        }
        if self.audio.sample_rate > 384_000 {
            issues.push("audio.sample_rate must be at most 384000".to_string());
        }

        if self.audio.channels > 0 && self.audio.channels > 32 {
            issues.push("audio.channels must be 1..=32 when set".to_string());
        }

        if self.audio.max_duration_secs > 86_400 {
            issues.push("audio.max_duration_secs must be at most 86,400".to_string());
        }

        if self.output.paste_delay_ms > 5_000 {
            issues.push("output.paste_delay_ms must be at most 5000".to_string());
        }

        if self.output.processing_timeout_ms > 600_000 {
            issues.push("output.processing_timeout_ms must be at most 600,000".to_string());
        }

        if self.provider.openrouter.max_audio_bytes == 0 {
            issues.push("provider.openrouter.max_audio_bytes must be greater than 0".to_string());
        } else if self.provider.openrouter.max_audio_bytes > 512 * 1024 * 1024 {
            issues.push("provider.openrouter.max_audio_bytes must be at most 536,870,912".to_string());
        }

        if self.tray.refresh_ms < 100 {
            issues.push("tray.refresh_ms must be at least 100".to_string());
        }
        if self.tray.refresh_ms > 120_000 {
            issues.push("tray.refresh_ms must be at most 120,000".to_string());
        }

        if self.audio_cues.volume < 0.0 || self.audio_cues.volume > 2.0 {
            issues.push("audio_cues.volume must be between 0.0 and 2.0".to_string());
        }

        self.pipeline.validate_with_legacy(&mut issues, &self.interaction, &self.audio);

        if issues.is_empty() {
            Ok(())
        } else {
            let mut out = String::from("invalid config:");
            for issue in issues {
                out.push_str(&format!(" {issue};"));
            }
            Err(anyhow!("{}", out.trim_end_matches(';')))
        }
    }

    pub fn parse(raw: &str) -> Result<Self> {
        let migrated = migration::migrate(raw)?;
        let cfg: Self = serde_json::from_value(migrated).with_context(|| "invalid config schema")?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn load_from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        if raw.trim().is_empty() {
            return Err(anyhow!("{} is empty", path.display()));
        }
        Self::parse(&raw)
    }

    pub fn load_or_create() -> Result<Self> {
        Self::seed_default_sound_cues()?;

        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }

        if path.exists() {
            return Self::load_from_path(&path);
        }

        let cfg = Self::default();
        cfg.save_validated_to(&path)?;
        Ok(cfg)
    }

    fn seed_default_sound_cues() -> Result<()> {
        let target_dir = Self::base_dir().join("sounds");
        fs::create_dir_all(&target_dir)
            .with_context(|| format!("creating sound cue directory {}", target_dir.display()))?;

        let assets: [(&str, &[u8]); 3] = [
            ("start.wav", include_bytes!("../sounds/start.wav")),
            ("stop.wav", include_bytes!("../sounds/stop.wav")),
            ("error_1.wav", include_bytes!("../sounds/error_1.wav")),
        ];

        for (name, bytes) in assets {
            let target = target_dir.join(name);
            if !target.exists() {
                fs::write(&target, bytes)
                    .with_context(|| format!("writing sound cue {}", target.display()))?;
            }
        }

        Ok(())
    }

    pub fn save_to<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let payload = serde_json::to_string_pretty(self)?;
        fs::write(path, payload)?;
        Ok(())
    }

    pub fn save_validated_to<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        self.validate()?;
        self.save_to(path)
    }
}
