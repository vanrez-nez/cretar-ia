use crate::config::AppConfig;
use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

pub const SETTINGS_DB_URL: &str = "sqlite:cretar-ia.db";
const SETTINGS_KEY: &str = "app_config";

#[derive(Clone)]
pub struct SettingsDb {
    pool: SqlitePool,
    app_data_dir: PathBuf,
    db_path: PathBuf,
}

impl SettingsDb {
    pub async fn connect(app: &AppHandle) -> Result<Self> {
        let app_data_dir = app
            .path()
            .app_config_dir()
            .context("resolving app config directory")?;
        std::fs::create_dir_all(&app_data_dir)
            .with_context(|| format!("creating {}", app_data_dir.display()))?;
        AppConfig::set_app_data_dir(app_data_dir.clone());
        AppConfig::seed_default_sound_cues()?;

        let db_path = app_data_dir.join("cretar-ia.db");
        let url = format!("sqlite:{}", db_path.to_string_lossy());
        let pool = SqlitePool::connect(&url)
            .await
            .with_context(|| format!("connecting settings database {}", db_path.display()))?;

        Ok(Self {
            pool,
            app_data_dir,
            db_path,
        })
    }

    pub fn app_data_dir(&self) -> &Path {
        &self.app_data_dir
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub async fn load_config(&self) -> Result<AppConfig> {
        let row = sqlx::query("SELECT value FROM settings WHERE key = $1")
            .bind(SETTINGS_KEY)
            .fetch_optional(&self.pool)
            .await
            .context("loading app config from sqlite")?;

        let Some(row) = row else {
            let config = AppConfig::default();
            self.save_config(config.clone()).await?;
            return Ok(config);
        };

        let raw: String = row.try_get("value").context("reading app config value")?;
        AppConfig::parse(&raw)
    }

    pub async fn save_config(&self, config: AppConfig) -> Result<AppConfig> {
        config.validate()?;
        let value = serde_json::to_string_pretty(&config).context("serializing app config")?;
        let updated_at = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, $3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(SETTINGS_KEY)
        .bind(value)
        .bind(updated_at)
        .execute(&self.pool)
        .await
        .context("saving app config to sqlite")?;

        Ok(config)
    }
}
