use anyhow::{anyhow, Context, Result};
use dirs::home_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionMode {
    PushToTalk,
    Toggle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    ClipboardOnly,
    ClipboardPaste,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionConfig {
    pub mode: InteractionMode,
    pub shortcut: String,
    #[serde(default = "InteractionConfig::default_repeat_debounce_ms")]
    pub repeat_debounce_ms: u64,
}

impl Default for InteractionConfig {
    fn default() -> Self {
        Self {
            mode: InteractionMode::PushToTalk,
            shortcut: "ctrl+shift+space".to_string(),
            repeat_debounce_ms: Self::default_repeat_debounce_ms(),
        }
    }
}

impl InteractionConfig {
    fn default_repeat_debounce_ms() -> u64 {
        120
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
}

impl Default for OpenRouterConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "openai/whisper-1".to_string(),
            base_url: Self::default_base_url(),
            endpoint: Self::default_endpoint(),
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
    pub max_duration_secs: u64,
    pub recording_dir: String,
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            sample_rate: 0,
            channels: 0,
            input_device: None,
            max_duration_secs: 120,
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
}

impl OutputConfig {
    fn default_paste_delay() -> u64 {
        40
    }
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            mode: OutputMode::ClipboardPaste,
            paste_delay_ms: Self::default_paste_delay(),
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
        let p = PathBuf::from(rel);
        if p.is_absolute() {
            p
        } else {
            base.join(p)
        }
    }

    pub fn recordings_path(&self) -> PathBuf {
        Self::resolve_relative(&self.base_dir_path(), &self.audio.recording_dir)
    }

    pub fn resolve_sound_path(&self, value: Option<&String>) -> Option<PathBuf> {
        value
            .as_deref()
            .map(|v| Self::resolve_relative(&self.base_dir_path(), v))
    }

    pub fn log_file_path() -> PathBuf {
        Self::base_dir().join("logs").join("app.log")
    }

    pub fn load_or_create() -> Result<Self> {
        Self::seed_default_sound_cues()?;

        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }

        if path.exists() {
            let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            if raw.trim().is_empty() {
                return Err(anyhow!("{} is empty", path.display()));
            }
            match serde_json::from_str::<Self>(&raw) {
                Ok(cfg) => return Ok(cfg),
                Err(err) => {
                    println!("Invalid config {}; regenerating default file. Reason: {}", path.display(), err);
                }
            }
        }

        let cfg = Self::default();
        cfg.save_to(&path)?;
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
                fs::write(&target, bytes).with_context(|| format!("writing sound cue {}", target.display()))?;
            }
        }

        Ok(())
    }

    pub fn save_to<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let payload = serde_json::to_string_pretty(self)?;
        fs::write(path, payload)?;
        Ok(())
    }
}
