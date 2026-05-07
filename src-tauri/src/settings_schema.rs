use crate::config::{
    AppConfig, AudioCaptureConfig, AudioCueConfig, InteractionConfig, OpenRouterConfig,
    OutputConfig, PipelineConfig, ProviderConfig, RecoveryStrategyConfig, TrayConfig,
    TrayTooltipConfig, UiConfig,
};
use anyhow::{anyhow, Context, Result};
use jsonschema::JSONSchema;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

const SETTINGS_SCHEMA: &str = include_str!("../../src/settings/settings.schema.json");

pub fn default_settings() -> Result<Value> {
    let schema: Value = serde_json::from_str(SETTINGS_SCHEMA).context("parsing settings schema")?;
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .context("settings schema missing properties")?;
    let mut settings = Map::new();

    for (key, property) in properties {
        let default = property
            .get("default")
            .cloned()
            .with_context(|| format!("settings schema key '{key}' is missing a default"))?;
        settings.insert(key.clone(), default);
    }

    let settings = Value::Object(settings);
    validate_settings(&settings)?;
    Ok(settings)
}

pub fn validate_settings(settings: &Value) -> Result<()> {
    let schema: Value = serde_json::from_str(SETTINGS_SCHEMA).context("parsing settings schema")?;
    let compiled = JSONSchema::compile(&schema)
        .map_err(|err| anyhow!("compiling settings schema: {err}"))?;

    if let Err(errors) = compiled.validate(settings) {
        let messages = errors
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(anyhow!("invalid settings: {messages}"));
    }

    Ok(())
}

pub fn normalize_settings(settings: &mut Value) {
    normalize_sound_setting(
        settings,
        "recording.sounds.start",
        "sounds/start.wav",
        "sounds/sine_transition_start.wav",
    );
    normalize_sound_setting(
        settings,
        "recording.sounds.stop",
        "sounds/stop.wav",
        "sounds/sine_transition_stop.wav",
    );
    normalize_sound_setting(
        settings,
        "recording.sounds.error",
        "sounds/error_1.wav",
        "sounds/sine_error.wav",
    );
}

pub fn runtime_config_from_settings(settings: &Value) -> Result<AppConfig> {
    validate_settings(settings)?;

    let cfg = AppConfig {
        ui: UiConfig {
            language: setting(settings, "system.language")?,
        },
        provider: ProviderConfig {
            provider: setting(settings, "models.stt.provider")?,
            openrouter: OpenRouterConfig {
                api_key: setting(settings, "models.stt.openrouter.api_key")?,
                model: setting(settings, "models.stt.openrouter.model")?,
                base_url: setting(settings, "models.stt.openrouter.base_url")?,
                endpoint: setting(settings, "models.stt.openrouter.endpoint")?,
                prompt: setting(settings, "models.stt.openrouter.prompt")?,
                max_audio_bytes: setting(settings, "models.stt.openrouter.max_audio_bytes")?,
            },
        },
        interaction: InteractionConfig {
            mode: setting(settings, "recording.mode")?,
            shortcut: setting(settings, "recording.hotkey")?,
            repeat_debounce_ms: setting(settings, "recording.hotkey_repeat_debounce_ms")?,
            hotkey_queue_capacity: setting::<Option<u32>>(settings, "recording.processing.hotkey_queue_capacity")?
                .unwrap_or_default(),
            worker_queue_capacity: setting::<Option<u32>>(settings, "recording.processing.worker_queue_capacity")?
                .unwrap_or_default(),
        },
        pipeline: PipelineConfig {
            debounce_ms: setting(settings, "recording.processing.debounce_ms")?,
            settle_timeout_ms: setting(settings, "recording.processing.settle_timeout_ms")?,
            hotkey_queue_capacity: setting(settings, "recording.processing.hotkey_queue_capacity")?,
            worker_queue_capacity: setting(settings, "recording.processing.worker_queue_capacity")?,
            queue_saturation_policy: setting(settings, "recording.processing.queue_saturation_policy")?,
            max_recording_duration_secs: setting(settings, "recording.processing.max_recording_duration_secs")?,
            recovery: Some(RecoveryStrategyConfig {
                retry_start_timeout: setting(settings, "recording.recovery.retry_start_timeout")?,
                retry_stop_timeout: setting(settings, "recording.recovery.retry_stop_timeout")?,
                retry_processing_timeout: setting(settings, "recording.recovery.retry_processing_timeout")?,
                retry_queue_saturation: setting(settings, "recording.recovery.retry_queue_saturation")?,
            }),
        },
        audio: AudioCaptureConfig {
            sample_rate: setting(settings, "recording.microphone.sample_rate")?,
            channels: setting(settings, "recording.microphone.channels")?,
            input_device: setting(settings, "recording.microphone.input_device")?,
            auto_switch_to_primary_device: true,
            max_duration_secs: setting(settings, "recording.microphone.max_duration_secs")?,
            recording_dir: setting(settings, "recording.storage.recording_dir")?,
        },
        audio_cues: AudioCueConfig {
            enabled: setting(settings, "recording.sounds.enabled")?,
            start_sound: setting(settings, "recording.sounds.start")?,
            stop_sound: setting(settings, "recording.sounds.stop")?,
            error_sound: setting(settings, "recording.sounds.error")?,
            volume: setting(settings, "recording.sounds.volume")?,
        },
        output: OutputConfig {
            mode: setting(settings, "output.mode")?,
            paste_delay_ms: setting(settings, "output.paste_delay_ms")?,
            cleanup_recording_after_processing: !setting::<bool>(settings, "system.save_input_audio")?,
            processing_timeout_ms: setting(settings, "output.processing_timeout_ms")?,
        },
        tray: TrayConfig {
            title: setting(settings, "tray.title")?,
            icon: setting(settings, "tray.icon")?,
            refresh_ms: setting(settings, "tray.refresh_ms")?,
            tooltip: TrayTooltipConfig {
                idle: setting(settings, "tray.tooltip.idle")?,
                recording: setting(settings, "tray.tooltip.recording")?,
                sending: setting(settings, "tray.tooltip.sending")?,
                success: setting(settings, "tray.tooltip.success")?,
            },
        },
    };

    cfg.validate()?;
    Ok(cfg)
}

fn normalize_sound_setting(settings: &mut Value, key: &str, legacy: &str, next: &str) {
    let Some(value) = settings.get_mut(key) else {
        return;
    };

    if value.as_str() == Some(legacy) {
        *value = Value::String(next.to_string());
    }
}

fn setting<T>(settings: &Value, key: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_value(
        settings
            .get(key)
            .cloned()
            .with_context(|| format!("missing setting '{key}'"))?,
    )
    .with_context(|| format!("invalid setting '{key}'"))
}
