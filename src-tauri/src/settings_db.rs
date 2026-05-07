use crate::config::AppConfig;
use crate::settings_schema;
use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

pub const SETTINGS_DB_URL: &str = "sqlite:cretar-ia.db";

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

    pub async fn load_settings(&self) -> Result<serde_json::Value> {
        let mut settings = settings_schema::default_settings()?;
        let rows = sqlx::query("SELECT key, value FROM settings")
            .fetch_all(&self.pool)
            .await
            .context("loading settings from sqlite")?;

        for row in rows {
            let key: String = row.try_get("key").context("reading setting key")?;
            let Some(current) = settings.get_mut(&key) else {
                continue;
            };
            let raw: String = row.try_get("value").context("reading setting value")?;
            *current = serde_json::from_str(&raw).unwrap_or(serde_json::Value::String(raw));
        }

        settings_schema::normalize_settings(&mut settings);
        settings_schema::validate_settings(&settings)?;
        self.save_settings(&settings).await?;
        Ok(settings)
    }

    pub async fn save_settings(&self, settings: &serde_json::Value) -> Result<()> {
        settings_schema::validate_settings(settings)?;
        let updated_at = chrono::Utc::now().to_rfc3339();

        let Some(values) = settings.as_object() else {
            return Err(anyhow::anyhow!("settings must be an object"));
        };

        for (key, value) in values {
            self.upsert_setting(key, value.clone(), &updated_at).await?;
        }

        Ok(())
    }

    pub async fn upsert_setting(
        &self,
        key: &str,
        value: serde_json::Value,
        updated_at: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, $3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(serde_json::to_string(&value).context("serializing setting value")?)
        .bind(updated_at)
        .execute(&self.pool)
        .await
        .with_context(|| format!("saving setting '{key}' to sqlite"))?;

        Ok(())
    }

    pub async fn load_config(&self) -> Result<AppConfig> {
        let settings = self.load_settings().await?;
        settings_schema::runtime_config_from_settings(&settings)
    }
}
