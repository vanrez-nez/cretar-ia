use crate::config::AppConfig;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SettingsService {
    config_path: PathBuf,
}

impl SettingsService {
    pub fn new(config_path: impl Into<PathBuf>) -> Self {
        Self {
            config_path: config_path.into(),
        }
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn prepare_default() -> Result<Self> {
        let path = AppConfig::config_path();
        AppConfig::load_or_create()
            .with_context(|| format!("preparing config at {}", path.display()))?;
        Ok(Self::new(path))
    }

    pub fn load(&self) -> Result<AppConfig> {
        AppConfig::load_from_path(&self.config_path)
    }

    pub fn save_value(&self, config: Value) -> Result<()> {
        let config = serde_json::from_value::<AppConfig>(config)
            .with_context(|| "invalid config schema")?;
        self.save(config)
    }

    pub fn save(&self, config: AppConfig) -> Result<()> {
        config.save_validated_to(&self.config_path)
    }

    pub fn command_error(err: anyhow::Error) -> String {
        err.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_config_path(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("cretar-ia-{name}-{suffix}.json"))
    }

    #[test]
    fn load_reads_valid_config_from_service_path() {
        let path = temp_config_path("load");
        let cfg = AppConfig::default();
        cfg.save_validated_to(&path).expect("write config");

        let loaded = SettingsService::new(&path).load().expect("load config");

        assert_eq!(loaded.audio.sample_rate, cfg.audio.sample_rate);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn save_rejects_invalid_config_without_overwriting_file() {
        let path = temp_config_path("invalid");
        let cfg = AppConfig::default();
        cfg.save_validated_to(&path).expect("write config");

        let mut invalid = cfg.clone();
        invalid.audio.sample_rate = 1;
        let err = SettingsService::new(&path)
            .save(invalid)
            .expect_err("invalid config rejected");

        assert!(err.to_string().contains("audio.sample_rate"));
        let loaded = AppConfig::load_from_path(&path).expect("original config remains valid");
        assert_eq!(loaded.audio.sample_rate, cfg.audio.sample_rate);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn save_value_uses_same_validation_path_as_typed_update() {
        let path = temp_config_path("value");
        let mut value = serde_json::to_value(AppConfig::default()).expect("serialize config");
        value["output"]["paste_delay_ms"] = serde_json::json!(6000);

        let err = SettingsService::new(&path)
            .save_value(value)
            .expect_err("invalid value rejected");

        assert!(err.to_string().contains("output.paste_delay_ms"));
        assert!(!path.exists());
    }
}
