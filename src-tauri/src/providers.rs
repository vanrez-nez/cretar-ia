use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use crate::model_health::{ModelHealthCache, ModelHealthStatus};
use jsonschema::JSONSchema;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

const PROVIDER_PRESETS: &str = include_str!("../providers/presets.json");
const PROVIDER_BODY_PREVIEW_LIMIT: usize = 4096;
const PROVIDER_MODEL_SAMPLE_LIMIT: usize = 10;

pub type DynSpeechToTextProvider = Arc<dyn SpeechToTextProvider>;
pub type DynFormattingProvider = Arc<dyn FormattingProvider>;

type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub trait SpeechToTextProvider: Send + Sync {
    fn transcribe<'a>(&'a self, wav_file: &'a Path) -> ProviderFuture<'a, String>;
}

pub trait FormattingProvider: Send + Sync {
    fn format<'a>(&'a self, text: &'a str) -> ProviderFuture<'a, String>;
}

#[derive(Clone)]
pub struct ProviderFactory {
    pool: SqlitePool,
    client: Client,
    health_cache: Option<ModelHealthCache>,
    app: Option<AppHandle>,
}

impl ProviderFactory {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            client: Client::new(),
            health_cache: None,
            app: None,
        }
    }

    pub fn with_health(pool: SqlitePool, health_cache: ModelHealthCache, app: AppHandle) -> Self {
        Self {
            pool,
            client: Client::new(),
            health_cache: Some(health_cache),
            app: Some(app),
        }
    }

    pub async fn speech_to_text(&self) -> Result<Option<DynSpeechToTextProvider>> {
        let Some(selection) = self.load_active_model("stt").await? else {
            return Ok(None);
        };

        validate_json(
            &selection.provider_config_schema,
            &selection.provider_config,
            &format!("provider '{}' config", selection.provider_id),
        )?;
        validate_json(
            &selection.model_request_config_schema,
            &selection.model_request_config,
            &format!("model '{}' request config", selection.model_id),
        )?;
        validate_json(
            &selection.model_adapter_config_schema,
            &selection.model_adapter_config,
            &format!("model '{}' adapter config", selection.model_id),
        )?;

        let mut runtime_model_config = selection.model_adapter_config.clone();
        merge_config_values(&mut runtime_model_config, &selection.model_request_config);
        let provider_config: RuntimeProviderConfig = serde_json::from_value(selection.provider_config.clone())
            .with_context(|| format!("parsing provider '{}' runtime config", selection.provider_id))?;
        let model_config: RuntimeModelConfig = serde_json::from_value(runtime_model_config)
            .with_context(|| format!("parsing model '{}' runtime config", selection.model_id))?;

        let driver = SttDriver::from_config(&model_config)?;
        log::info!(
            "provider factory loaded stt provider={} model={} driver={}",
            selection.provider_id,
            selection.external_model_id,
            driver.as_str()
        );

        let user_model_id = selection.user_model_id.clone();
        Ok(Some(Arc::new(GenericSpeechToTextProvider {
            client: self.client.clone(),
            user_model_id: user_model_id.clone(),
            provider_id: selection.provider_id,
            model_id: selection.model_id,
            external_model_id: selection.external_model_id,
            provider_config,
            model_config,
            driver,
            health_reporter: self.health_reporter("stt", &user_model_id),
        })))
    }

    pub async fn formatting(&self) -> Result<Option<DynFormattingProvider>> {
        let Some(selection) = self.load_active_model("formatting").await? else {
            return Ok(None);
        };

        validate_json(
            &selection.provider_config_schema,
            &selection.provider_config,
            &format!("provider '{}' config", selection.provider_id),
        )?;
        validate_json(
            &selection.model_request_config_schema,
            &selection.model_request_config,
            &format!("model '{}' request config", selection.model_id),
        )?;
        validate_json(
            &selection.model_adapter_config_schema,
            &selection.model_adapter_config,
            &format!("model '{}' adapter config", selection.model_id),
        )?;

        let mut runtime_model_config = selection.model_adapter_config.clone();
        merge_config_values(&mut runtime_model_config, &selection.model_request_config);
        let provider_config: RuntimeProviderConfig = serde_json::from_value(selection.provider_config.clone())
            .with_context(|| format!("parsing provider '{}' runtime config", selection.provider_id))?;
        let model_config: RuntimeModelConfig = serde_json::from_value(runtime_model_config)
            .with_context(|| format!("parsing model '{}' runtime config", selection.model_id))?;

        let user_model_id = selection.user_model_id.clone();
        Ok(Some(Arc::new(GenericFormattingProvider {
            client: self.client.clone(),
            user_model_id: user_model_id.clone(),
            provider_id: selection.provider_id,
            model_id: selection.model_id,
            external_model_id: selection.external_model_id,
            provider_config,
            model_config,
            health_reporter: self.health_reporter("formatting", &user_model_id),
        })))
    }

    fn health_reporter(&self, role: &str, user_model_id: &str) -> Option<ModelHealthReporter> {
        Some(ModelHealthReporter {
            cache: self.health_cache.clone()?,
            app: self.app.clone()?,
            role: role.to_string(),
            user_model_id: user_model_id.to_string(),
        })
    }

    async fn load_active_model(&self, role: &str) -> Result<Option<ProviderModelSelection>> {
        self.load_user_model_row(role, "AND um.is_active = 1", None).await
    }

    async fn load_user_model_row(
        &self,
        role: &str,
        extra_where: &str,
        user_model_id: Option<&str>,
    ) -> Result<Option<ProviderModelSelection>> {
        let sql = format!(
            "SELECT
                um.id AS user_model_id,
                p.id AS provider_id,
                p.config_json AS provider_config_json,
                p.config_schema_json AS provider_config_schema_json,
                m.id AS model_id,
                m.external_model_id AS external_model_id,
                m.config_json AS model_config_json,
                m.config_schema_json AS model_config_schema_json,
                m.adapter_config_json AS model_adapter_config_json,
                m.adapter_config_schema_json AS model_adapter_config_schema_json,
                um.provider_config_override_json AS provider_override_json,
                um.model_config_override_json AS model_override_json
             FROM user_models um
             JOIN providers p ON p.id = um.provider_id
             JOIN models m ON m.id = um.model_id
             WHERE um.role = $1
                AND p.enabled = 1
                AND m.enabled = 1
                {extra_where}
             ORDER BY um.is_active DESC, um.updated_at DESC
             LIMIT 1"
        );
        let mut query = sqlx::query(&sql).bind(role);
        if let Some(user_model_id) = user_model_id {
            query = query.bind(user_model_id);
        }
        let row = query
            .fetch_optional(&self.pool)
            .await
            .with_context(|| format!("loading {role} user model"))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let mut provider_config = parse_json_column(&row, "provider_config_json")?;
        let provider_override = parse_json_column(&row, "provider_override_json")?;
        merge_config_values(&mut provider_config, &provider_override);
        let model_request_config = parse_json_column(&row, "model_override_json")?;
        let model_adapter_config = parse_json_column(&row, "model_adapter_config_json")?;
        Ok(Some(ProviderModelSelection {
            user_model_id: row.try_get("user_model_id").context("reading user_model_id")?,
            provider_id: row.try_get("provider_id").context("reading provider_id")?,
            provider_config,
            provider_config_schema: parse_json_column(&row, "provider_config_schema_json")?,
            provider_override,
            model_id: row.try_get("model_id").context("reading model_id")?,
            external_model_id: row.try_get("external_model_id").context("reading external_model_id")?,
            model_request_config,
            model_request_config_schema: parse_json_column(&row, "model_config_schema_json")?,
            model_adapter_config,
            model_adapter_config_schema: parse_json_column(&row, "model_adapter_config_schema_json")?,
        }))
    }

    pub async fn model_settings(&self, role: &str) -> Result<RoleModelSettings> {
        validate_role(role)?;
        Ok(RoleModelSettings {
            role: role.to_string(),
            model_id: self.load_active_model_id(role).await?,
            providers: self.load_provider_views(role).await?,
            models: self.load_catalog_model_views(role).await?,
            user_models: self.load_user_model_views(role).await?,
        })
    }

    async fn load_active_model_id(&self, role: &str) -> Result<Option<String>> {
        let row = sqlx::query("SELECT id FROM user_models WHERE role = $1 AND is_active = 1 LIMIT 1")
            .bind(role)
            .fetch_optional(&self.pool)
            .await
            .with_context(|| format!("loading active user model for role '{role}'"))?;
        row.map(|row| row.try_get("id").context("reading active user model id"))
            .transpose()
    }

    async fn load_provider_views(&self, role: &str) -> Result<Vec<ProviderSettingsView>> {
        let rows = sqlx::query(
            "SELECT id, name, kind, config_json, config_schema_json
             FROM providers
             WHERE enabled = 1
             ORDER BY name ASC",
        )
        .fetch_all(&self.pool)
        .await
        .context("loading provider settings")?;

        let mut views = Vec::with_capacity(rows.len());
        for row in rows {
            let config = parse_json_column(&row, "config_json")?;
            if !provider_config_supports_role(&config, role) {
                continue;
            }
            views.push(ProviderSettingsView {
                id: row.try_get("id").context("reading provider id")?,
                name: row.try_get("name").context("reading provider name")?,
                kind: row.try_get("kind").context("reading provider kind")?,
                config: config.clone(),
                override_config: Value::Object(Map::new()),
                effective_config: config,
                config_schema: parse_json_column(&row, "config_schema_json")?,
            });
        }
        Ok(views)
    }

    async fn load_catalog_model_views(&self, role: &str) -> Result<Vec<CatalogModelView>> {
        let rows = sqlx::query(
            "SELECT id, provider_id, role, external_model_id, display_name, config_json, config_schema_json
             FROM models
             WHERE enabled = 1 AND role = $1
             ORDER BY display_name ASC",
        )
        .bind(role)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("loading catalog models for role '{role}'"))?;

        let mut views = Vec::with_capacity(rows.len());
        for row in rows {
            views.push(CatalogModelView {
                id: row.try_get("id").context("reading catalog model id")?,
                provider_id: row.try_get("provider_id").context("reading catalog model provider id")?,
                role: row.try_get("role").context("reading catalog model role")?,
                external_model_id: row.try_get("external_model_id").context("reading catalog external model id")?,
                display_name: row.try_get("display_name").context("reading catalog model display name")?,
                config: parse_json_column(&row, "config_json")?,
                config_schema: parse_json_column(&row, "config_schema_json")?,
            });
        }
        Ok(views)
    }

    async fn load_user_model_views(&self, role: &str) -> Result<Vec<UserModelView>> {
        let rows = sqlx::query(
            "SELECT
                um.id AS id,
                um.role AS role,
                um.provider_id AS provider_id,
                um.model_id AS model_id,
                um.provider_config_override_json AS provider_override_json,
                um.model_config_override_json AS model_override_json,
                um.is_active AS is_active,
                p.name AS provider_name,
                p.kind AS provider_kind,
                p.config_json AS provider_config_json,
                p.config_schema_json AS provider_config_schema_json,
                m.external_model_id AS external_model_id,
                m.display_name AS model_display_name,
                m.config_json AS model_config_json,
                m.config_schema_json AS model_config_schema_json
             FROM user_models um
             JOIN providers p ON p.id = um.provider_id
             JOIN models m ON m.id = um.model_id
             WHERE um.role = $1
             ORDER BY um.created_at ASC",
        )
        .bind(role)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("loading user models for role '{role}'"))?;

        let mut views = Vec::with_capacity(rows.len());
        for row in rows {
            let provider_config = parse_json_column(&row, "provider_config_json")?;
            let provider_override_config = parse_json_column(&row, "provider_override_json")?;
            let mut provider_effective_config = provider_config.clone();
            merge_config_values(&mut provider_effective_config, &provider_override_config);
            let config = parse_json_column(&row, "model_config_json")?;
            let override_config = parse_json_column(&row, "model_override_json")?;
            let mut effective_config = config.clone();
            merge_config_values(&mut effective_config, &override_config);
            views.push(UserModelView {
                id: row.try_get("id").context("reading user model id")?,
                role: row.try_get("role").context("reading user model role")?,
                provider_id: row.try_get("provider_id").context("reading user model provider id")?,
                provider_name: row.try_get("provider_name").context("reading user model provider name")?,
                provider_kind: row.try_get("provider_kind").context("reading user model provider kind")?,
                provider_config,
                provider_override_config,
                provider_effective_config,
                provider_config_schema: parse_json_column(&row, "provider_config_schema_json")?,
                model_id: row.try_get("model_id").context("reading user model catalog model id")?,
                external_model_id: row.try_get("external_model_id").context("reading user model external model id")?,
                model_display_name: row.try_get("model_display_name").context("reading user model catalog display name")?,
                config,
                override_config,
                effective_config,
                config_schema: parse_json_column(&row, "model_config_schema_json")?,
                is_active: row.try_get::<i64, _>("is_active").context("reading user model active flag")? == 1,
            });
        }
        Ok(views)
    }

    pub async fn refresh_provider_models(
        &self,
        role: &str,
        provider_id: &str,
        provider_override_config: Value,
    ) -> Result<Vec<ProviderModelOption>> {
        validate_role(role)?;
        validate_override_object(&provider_override_config)?;
        let row = sqlx::query(
            "SELECT config_json, config_schema_json FROM providers WHERE id = $1 AND enabled = 1",
        )
        .bind(provider_id)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("loading provider '{provider_id}'"))?;
        let mut config = parse_json_column(&row, "config_json")?;
        let schema = parse_json_column(&row, "config_schema_json")?;
        merge_config_values(&mut config, &provider_override_config);
        validate_json(&schema, &config, &format!("provider '{provider_id}' refresh config"))?;
        ensure_provider_supports_role(&config, role, provider_id)?;
        let provider_config: RuntimeProviderConfig = serde_json::from_value(config.clone())
            .with_context(|| format!("parsing provider '{provider_id}' refresh config"))?;
        validate_runtime_provider_auth(&provider_config)?;
        let strategy = config
            .pointer("/model_fetch/strategy")
            .and_then(Value::as_str)
            .unwrap_or("manual");
        let query_params = model_fetch_query_params(&config, role);

        let options = match strategy {
            "openrouter_models" | "openai_models" | "ollama_openai_models" | "ollama_tags" => {
                fetch_configured_models(
                    &self.client,
                    provider_id,
                    &provider_config,
                    &config,
                    role,
                    &query_params,
                )
                .await?
            }
            _ => Vec::new(),
        };
        self.upsert_catalog_models(role, provider_id, &options).await
    }

    async fn upsert_catalog_models(
        &self,
        role: &str,
        provider_id: &str,
        options: &[ProviderModelOption],
    ) -> Result<Vec<ProviderModelOption>> {
        let (request_config, request_schema, adapter_config, adapter_schema) =
            operation_template_for_role(&self.pool, provider_id, role).await?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut saved_options = Vec::with_capacity(options.len());
        for option in options {
            let model_id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO models (
                    id, provider_id, role, external_model_id, display_name, config_json,
                    config_schema_json, adapter_config_json, adapter_config_schema_json,
                    enabled, is_preset, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1, 0, $10, $10)
                 ON CONFLICT(provider_id, external_model_id, role) DO UPDATE SET
                    display_name = excluded.display_name,
                    config_json = excluded.config_json,
                    config_schema_json = excluded.config_schema_json,
                    adapter_config_json = excluded.adapter_config_json,
                    adapter_config_schema_json = excluded.adapter_config_schema_json,
                    updated_at = excluded.updated_at",
            )
            .bind(model_id)
            .bind(provider_id)
            .bind(role)
            .bind(&option.id)
            .bind(&option.name)
            .bind(serde_json::to_string(&request_config).context("serializing catalog model request config")?)
            .bind(serde_json::to_string(&request_schema).context("serializing catalog model request schema")?)
            .bind(serde_json::to_string(&adapter_config).context("serializing catalog model adapter config")?)
            .bind(serde_json::to_string(&adapter_schema).context("serializing catalog model adapter schema")?)
            .bind(&now)
            .execute(&self.pool)
            .await
            .with_context(|| format!("saving catalog model '{}'", option.id))?;
            let row = sqlx::query(
                "SELECT id FROM models WHERE provider_id = $1 AND external_model_id = $2 AND role = $3",
            )
            .bind(provider_id)
            .bind(&option.id)
            .bind(role)
            .fetch_one(&self.pool)
            .await
            .with_context(|| format!("loading saved catalog model '{}'", option.id))?;
            saved_options.push(ProviderModelOption {
                id: row.try_get("id").context("reading saved catalog model id")?,
                name: option.name.clone(),
            });
        }
        Ok(saved_options)
    }

    pub async fn save_model_item(
        &self,
        user_model_id: Option<String>,
        role: &str,
        provider_id: &str,
        model_id: &str,
        provider_override_config: Value,
        model_override_config: Value,
    ) -> Result<String> {
        validate_role(role)?;
        validate_override_object(&provider_override_config)?;
        validate_override_object(&model_override_config)?;
        if model_id.trim().is_empty() {
            return Err(anyhow!("model id is required"));
        }
        let row = sqlx::query(
            "SELECT
                p.config_json AS provider_config_json,
                p.config_schema_json AS provider_config_schema_json,
                m.config_json AS model_config_json,
                m.config_schema_json AS model_config_schema_json
             FROM models m
             JOIN providers p ON p.id = m.provider_id
             WHERE m.id = $1 AND m.provider_id = $2 AND m.role = $3 AND m.enabled = 1 AND p.enabled = 1",
        )
        .bind(model_id)
        .bind(provider_id)
        .bind(role)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("loading catalog model '{model_id}'"))?;

        let provider_schema = parse_json_column(&row, "provider_config_schema_json")?;
        validate_json(&provider_schema, &provider_override_config, &format!("provider '{provider_id}' settings config"))?;
        ensure_provider_supports_role(&provider_override_config, role, provider_id)?;
        let provider_config: RuntimeProviderConfig = serde_json::from_value(provider_override_config.clone())
            .with_context(|| format!("parsing provider '{provider_id}' settings config"))?;
        validate_runtime_provider_auth(&provider_config)?;

        let model_schema = parse_json_column(&row, "model_config_schema_json")?;
        validate_json(&model_schema, &model_override_config, &format!("model '{model_id}' request config"))?;

        let now = chrono::Utc::now().to_rfc3339();
        let user_model_id = if let Some(user_model_id) = user_model_id {
            let exists: i64 = sqlx::query("SELECT COUNT(*) AS count FROM user_models WHERE id = $1 AND role = $2")
                .bind(&user_model_id)
                .bind(role)
                .fetch_one(&self.pool)
                .await
                .with_context(|| format!("loading user model '{user_model_id}'"))?
                .try_get("count")
                .context("reading user model count")?;
            if exists == 0 {
                return Err(anyhow!("model item '{user_model_id}' was not found for role '{role}'"));
            }
            user_model_id
        } else {
            Uuid::new_v4().to_string()
        };
        sqlx::query(
            "INSERT INTO user_models (
                id, role, provider_id, model_id,
                provider_config_override_json, model_config_override_json,
                is_active, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, 0, $7, $7)
             ON CONFLICT(id) DO UPDATE SET
                provider_id = excluded.provider_id,
                model_id = excluded.model_id,
                provider_config_override_json = excluded.provider_config_override_json,
                model_config_override_json = excluded.model_config_override_json,
                updated_at = excluded.updated_at",
        )
        .bind(&user_model_id)
        .bind(role)
        .bind(provider_id)
        .bind(model_id)
        .bind(serde_json::to_string(&provider_override_config).context("serializing provider override config")?)
        .bind(serde_json::to_string(&model_override_config).context("serializing model override config")?)
        .bind(&now)
        .execute(&self.pool)
        .await
        .with_context(|| format!("saving user model '{user_model_id}'"))?;

        self.set_active_model(role, &user_model_id).await?;

        Ok(user_model_id)
    }

    pub async fn set_active_model(&self, role: &str, user_model_id: &str) -> Result<()> {
        validate_role(role)?;
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("UPDATE user_models SET is_active = 0, updated_at = $1 WHERE role = $2")
            .bind(&now)
            .bind(role)
            .execute(&self.pool)
            .await
            .with_context(|| format!("clearing active user models for role '{role}'"))?;
        let result = sqlx::query("UPDATE user_models SET is_active = 1, updated_at = $1 WHERE id = $2 AND role = $3")
            .bind(&now)
            .bind(user_model_id)
            .bind(role)
            .execute(&self.pool)
            .await
            .with_context(|| format!("activating user model '{user_model_id}'"))?;
        if result.rows_affected() == 0 {
            return Err(anyhow!("model item '{user_model_id}' was not found for role '{role}'"));
        }
        Ok(())
    }

    pub async fn delete_model_item(&self, role: &str, user_model_id: &str) -> Result<bool> {
        validate_role(role)?;
        let deleted_row = sqlx::query("SELECT is_active FROM user_models WHERE role = $1 AND id = $2")
            .bind(role)
            .bind(user_model_id)
            .fetch_optional(&self.pool)
            .await
            .with_context(|| format!("loading user model '{user_model_id}' before delete"))?;
        let Some(deleted_row) = deleted_row else {
            return Ok(false);
        };
        let was_active = deleted_row
            .try_get::<i64, _>("is_active")
            .context("reading deleted user model active flag")?
            == 1;

        let result = sqlx::query("DELETE FROM user_models WHERE role = $1 AND id = $2")
            .bind(role)
            .bind(user_model_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("deleting user model '{user_model_id}'"))?;

        if was_active {
            if let Some(next_id) = self.next_user_model_id(role).await? {
                self.set_active_model(role, &next_id).await?;
            }
        }

        Ok(result.rows_affected() > 0)
    }

    async fn next_user_model_id(&self, role: &str) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT id
             FROM user_models
             WHERE role = $1
             ORDER BY created_at ASC
             LIMIT 1",
        )
        .bind(role)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("loading next user model for role '{role}'"))?;
        row.map(|row| row.try_get("id").context("reading next user model id"))
            .transpose()
    }
}

#[derive(Debug)]
struct ProviderModelSelection {
    user_model_id: String,
    provider_id: String,
    provider_config: Value,
    provider_config_schema: Value,
    provider_override: Value,
    model_id: String,
    external_model_id: String,
    model_request_config: Value,
    model_request_config_schema: Value,
    model_adapter_config: Value,
    model_adapter_config_schema: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeProviderConfig {
    base_url: String,
    auth: AuthConfig,
    endpoints: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelFetchResponseConfig {
    items_path: String,
    id_path: String,
    name_path: String,
    #[serde(default)]
    fallback_name_path: Option<String>,
    #[serde(default)]
    filters_by_role: HashMap<String, ModelFetchRoleFilter>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ModelFetchRoleFilter {
    #[serde(default)]
    include_exact: Vec<String>,
    #[serde(default)]
    include_prefix: Vec<String>,
    #[serde(default)]
    include_contains: Vec<String>,
    #[serde(default)]
    exclude_exact: Vec<String>,
    #[serde(default)]
    exclude_prefix: Vec<String>,
    #[serde(default)]
    exclude_contains: Vec<String>,
}

struct ProviderRequestLogContext {
    operation: &'static str,
    provider_id: String,
    user_model_id: Option<String>,
    model_id: Option<String>,
    external_model_id: Option<String>,
    url: String,
    started_at: std::time::Instant,
}

impl ProviderRequestLogContext {
    fn new(operation: &'static str, provider_id: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            operation,
            provider_id: provider_id.into(),
            user_model_id: None,
            model_id: None,
            external_model_id: None,
            url: url.into(),
            started_at: std::time::Instant::now(),
        }
    }

    fn user_model_id(mut self, user_model_id: impl Into<String>) -> Self {
        self.user_model_id = Some(user_model_id.into());
        self
    }

    fn model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    fn external_model_id(mut self, external_model_id: impl Into<String>) -> Self {
        self.external_model_id = Some(external_model_id.into());
        self
    }

    fn elapsed_ms(&self) -> u128 {
        self.started_at.elapsed().as_millis()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct AuthConfig {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeModelConfig {
    #[serde(default)]
    operation_driver: Option<String>,
    #[serde(default)]
    endpoint_kind: Option<String>,
    #[serde(default = "default_audio_format")]
    format: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default = "default_max_audio_bytes")]
    max_audio_bytes: u64,
    #[serde(default)]
    response_text_paths: Vec<String>,
    #[serde(default)]
    parameters: Map<String, Value>,
}

fn default_audio_format() -> String {
    "wav".to_string()
}

fn default_max_audio_bytes() -> u64 {
    24 * 1024 * 1024
}

#[derive(Debug, Clone, Copy)]
enum SttDriver {
    HttpJsonAudioTranscription,
    MultipartAudioTranscription,
}

#[derive(Clone)]
struct ModelHealthReporter {
    cache: ModelHealthCache,
    app: AppHandle,
    role: String,
    user_model_id: String,
}

impl ModelHealthReporter {
    fn record_success(&self) {
        self.record(ModelHealthStatus::Healthy);
    }

    fn record_failure(&self) {
        self.record(ModelHealthStatus::Unhealthy);
    }

    fn record(&self, health: ModelHealthStatus) {
        if !self
            .cache
            .record_model_health_outcome(&self.role, &self.user_model_id, health)
        {
            log::debug!(
                "model health outcome ignored role={} user_model_id={} health={health:?} reason=not_in_cache",
                self.role,
                self.user_model_id
            );
            return;
        }
        log::debug!(
            "model health outcome recorded role={} user_model_id={} health={health:?}",
            self.role,
            self.user_model_id
        );
        notify_model_health_changed(&self.app);
    }
}

fn notify_model_health_changed(app: &AppHandle) {
    if let Some(config) = crate::app_host::current_config(app) {
        crate::app_host::refresh_tray_menu(app, &config);
    }
    if let Err(err) = app.emit(
        "settings:changed",
        serde_json::json!({
            "source": "model_health",
            "keys": ["models.health"],
        }),
    ) {
        log::warn!("failed to emit model health change: {err}");
    }
}

impl SttDriver {
    fn from_config(config: &RuntimeModelConfig) -> Result<Self> {
        match config.operation_driver.as_deref() {
            Some("http_json_audio_transcription") => Ok(Self::HttpJsonAudioTranscription),
            Some("multipart_audio_transcription") => Ok(Self::MultipartAudioTranscription),
            Some(other) => Err(anyhow!("unsupported stt operation_driver '{other}'")),
            None => Err(anyhow!("stt model config missing operation_driver")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::HttpJsonAudioTranscription => "http_json_audio_transcription",
            Self::MultipartAudioTranscription => "multipart_audio_transcription",
        }
    }
}

struct GenericSpeechToTextProvider {
    client: Client,
    user_model_id: String,
    provider_id: String,
    model_id: String,
    external_model_id: String,
    provider_config: RuntimeProviderConfig,
    model_config: RuntimeModelConfig,
    driver: SttDriver,
    health_reporter: Option<ModelHealthReporter>,
}

impl SpeechToTextProvider for GenericSpeechToTextProvider {
    fn transcribe<'a>(&'a self, wav_file: &'a Path) -> ProviderFuture<'a, String> {
        Box::pin(async move {
            let result = async {
                let start = std::time::Instant::now();
                let file_bytes = read_audio_file(wav_file, self.model_config.max_audio_bytes)?;
                let url = self.endpoint_url()?;

                let response = match self.driver {
                    SttDriver::HttpJsonAudioTranscription => self.send_json_audio(&url, &file_bytes).await?,
                    SttDriver::MultipartAudioTranscription => self.send_multipart_audio(&url, file_bytes).await?,
                };

                let text = extract_text(&response, &self.response_text_paths())?.trim().to_string();
                if text.is_empty() {
                    return Err(anyhow!("stt response text is empty"));
                }
                log::info!(
                    "stt completed provider={} user_model_id={} model_id={} model={} elapsed={:?} text_len={}",
                    self.provider_id,
                    self.user_model_id,
                    self.model_id,
                    self.external_model_id,
                    start.elapsed(),
                    text.len()
                );
                Ok(text)
            }
            .await;

            if let Some(reporter) = &self.health_reporter {
                if result.is_ok() {
                    reporter.record_success();
                } else {
                    reporter.record_failure();
                }
            }
            result
        })
    }
}

impl GenericSpeechToTextProvider {
    fn endpoint_url(&self) -> Result<String> {
        let endpoint_kind = self
            .model_config
            .endpoint_kind
            .as_deref()
            .unwrap_or("audio_transcriptions");
        let endpoint = self
            .provider_config
            .endpoints
            .get(endpoint_kind)
            .with_context(|| format!("provider '{}' missing endpoint '{endpoint_kind}'", self.provider_id))?;
        Ok(join_url(&self.provider_config.base_url, endpoint))
    }

    fn response_text_paths(&self) -> Vec<&str> {
        if self.model_config.response_text_paths.is_empty() {
            vec!["/text", "/choices/0/text"]
        } else {
            self.model_config
                .response_text_paths
                .iter()
                .map(String::as_str)
                .collect()
        }
    }

    async fn send_json_audio(&self, url: &str, file_bytes: &[u8]) -> Result<Value> {
        let mut payload = json!({
            "model": self.external_model_id,
            "input_audio": {
                "format": self.model_config.format,
                "data": encode_base64(file_bytes)
            }
        });
        insert_optional_string(&mut payload, "prompt", self.model_config.prompt.as_deref());
        insert_optional_string(&mut payload, "language", self.model_config.language.as_deref());
        insert_parameters(&mut payload, &self.model_config.parameters);

        let context = ProviderRequestLogContext::new("stt_json", &self.provider_id, url)
            .user_model_id(&self.user_model_id)
            .model_id(&self.model_id)
            .external_model_id(&self.external_model_id);
        log_raw_provider_request(
            &context,
            json!({
                "method": "POST",
                "url": url,
                "headers": provider_request_headers_for_log(&self.provider_config.auth, "application/json"),
                "body": payload.clone()
            }),
        );
        let request = apply_auth(self.client.post(url).json(&payload), &self.provider_config.auth)?;
        let response = request.send().await.context("posting json stt request")?;
        read_json_response(response, &context).await
    }

    async fn send_multipart_audio(&self, url: &str, file_bytes: Vec<u8>) -> Result<Value> {
        let audio_bytes = file_bytes.len();
        let part = reqwest::multipart::Part::bytes(file_bytes).file_name("recording.wav");
        let mut form = reqwest::multipart::Form::new()
            .text("model", self.external_model_id.clone())
            .part("file", part);
        if let Some(prompt) = self.model_config.prompt.as_deref().filter(|value| !value.is_empty()) {
            form = form.text("prompt", prompt.to_string());
        }
        if let Some(language) = self.model_config.language.as_deref().filter(|value| !value.is_empty()) {
            form = form.text("language", language.to_string());
        }
        for (key, value) in &self.model_config.parameters {
            if let Some(text) = value.as_str() {
                form = form.text(key.clone(), text.to_string());
            }
        }

        let context = ProviderRequestLogContext::new("stt_multipart", &self.provider_id, url)
            .user_model_id(&self.user_model_id)
            .model_id(&self.model_id)
            .external_model_id(&self.external_model_id);
        log_raw_provider_request(
            &context,
            json!({
                "method": "POST",
                "url": url,
                "headers": provider_request_headers_for_log(&self.provider_config.auth, "multipart/form-data"),
                "body": {
                    "multipart": true,
                    "fields": {
                        "model": self.external_model_id.clone(),
                        "prompt": self.model_config.prompt.clone(),
                        "language": self.model_config.language.clone(),
                        "parameters": self.model_config.parameters.clone()
                    },
                    "files": [
                        {
                            "field": "file",
                            "filename": "recording.wav",
                            "bytes": audio_bytes
                        }
                    ]
                }
            }),
        );
        let request = apply_auth(self.client.post(url).multipart(form), &self.provider_config.auth)?;
        let response = request.send().await.context("posting multipart stt request")?;
        read_json_response(response, &context).await
    }
}

struct GenericFormattingProvider {
    client: Client,
    user_model_id: String,
    provider_id: String,
    model_id: String,
    external_model_id: String,
    provider_config: RuntimeProviderConfig,
    model_config: RuntimeModelConfig,
    health_reporter: Option<ModelHealthReporter>,
}

impl FormattingProvider for GenericFormattingProvider {
    fn format<'a>(&'a self, text: &'a str) -> ProviderFuture<'a, String> {
        Box::pin(async move {
            let _ = (&self.client, &self.provider_config, &self.model_config, text);
            let result: Result<String> = Err(anyhow!(
                "formatting provider '{}' model_id='{}' model '{}' is loaded but formatting runtime is not wired yet",
                self.provider_id,
                self.model_id,
                self.external_model_id
            ));
            if let Some(reporter) = &self.health_reporter {
                reporter.record_failure();
            }
            result
        })
    }
}

#[derive(Debug, Deserialize)]
struct ProviderPresets {
    schema_version: u32,
    providers: Vec<ProviderPreset>,
    models: Vec<ModelPreset>,
}

#[derive(Debug, Deserialize)]
struct ProviderPreset {
    id: String,
    key: String,
    name: String,
    kind: String,
    config: Value,
    config_schema: Value,
    operation_templates: HashMap<String, OperationTemplatePreset>,
}

#[derive(Debug, Deserialize)]
struct ModelPreset {
    id: String,
    provider_key: String,
    role: String,
    external_model_id: String,
    display_name: String,
    config: Value,
    config_schema: Value,
}

#[derive(Debug, Deserialize)]
struct OperationTemplatePreset {
    request_config: Value,
    request_config_schema: Value,
    adapter_config: Value,
    adapter_config_schema: Value,
}

fn load_provider_presets() -> Result<ProviderPresets> {
    let presets: ProviderPresets =
        serde_json::from_str(PROVIDER_PRESETS).context("parsing provider presets")?;
    validate_presets(&presets)?;
    Ok(presets)
}

pub async fn seed_provider_presets(pool: &SqlitePool) -> Result<()> {
    let presets = load_provider_presets()?;

    let now = chrono::Utc::now().to_rfc3339();
    for provider in &presets.providers {
        sqlx::query(
            "INSERT INTO providers (
                id, key, name, kind, config_json, config_schema_json, enabled, is_preset, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, 1, 1, $7, $7)
             ON CONFLICT(key) DO UPDATE SET
                name = excluded.name,
                kind = excluded.kind,
                config_json = excluded.config_json,
                config_schema_json = excluded.config_schema_json,
                is_preset = 1,
                updated_at = excluded.updated_at",
        )
        .bind(&provider.id)
        .bind(&provider.key)
        .bind(&provider.name)
        .bind(&provider.kind)
        .bind(serde_json::to_string(&provider.config).context("serializing provider config")?)
        .bind(
            serde_json::to_string(&provider.config_schema)
                .context("serializing provider config schema")?,
        )
        .bind(&now)
        .execute(pool)
        .await
        .with_context(|| format!("seeding provider preset '{}'", provider.key))?;
        merge_config_defaults(pool, "providers", &provider.id, &provider.config).await?;
    }

    let seeded_models = 0usize;
    if !presets.models.is_empty() {
        log::warn!(
            "model presets are ignored; provider model catalog is populated only by refresh models={}",
            presets.models.len()
        );
    }



    log::info!(
        "provider presets seeded providers={} models={} schema_version={}",
        presets.providers.len(),
        seeded_models,
        presets.schema_version
    );
    Ok(())
}

fn validate_presets(presets: &ProviderPresets) -> Result<()> {
    if presets.schema_version != 2 {
        return Err(anyhow!(
            "unsupported provider presets schema_version {}",
            presets.schema_version
        ));
    }

    let mut provider_ids = HashSet::new();
    let mut provider_keys = HashSet::new();
    for provider in &presets.providers {
        Uuid::parse_str(&provider.id)
            .with_context(|| format!("provider preset '{}' id must be a UUID", provider.key))?;
        if !provider_ids.insert(provider.id.as_str()) {
            return Err(anyhow!("duplicate provider preset id '{}'", provider.id));
        }
        if !provider_keys.insert(provider.key.as_str()) {
            return Err(anyhow!("duplicate provider preset key '{}'", provider.key));
        }
        validate_json(
            &provider.config_schema,
            &provider.config,
            &format!("provider preset '{}' config", provider.key),
        )?;
        for (role, template) in &provider.operation_templates {
            if !matches!(role.as_str(), "stt" | "formatting") {
                return Err(anyhow!(
                    "provider preset '{}' has invalid operation template role '{}'",
                    provider.key,
                    role
                ));
            }
            validate_json(
                &template.request_config_schema,
                &template.request_config,
                &format!("provider preset '{}' role '{}' request template", provider.key, role),
            )?;
            validate_json(
                &template.adapter_config_schema,
                &template.adapter_config,
                &format!("provider preset '{}' role '{}' adapter template", provider.key, role),
            )?;
        }
    }

    let mut model_ids = HashSet::new();
    for model in &presets.models {
        Uuid::parse_str(&model.id)
            .with_context(|| format!("model preset '{}' id must be a UUID", model.id))?;
        if !model_ids.insert(model.id.as_str()) {
            return Err(anyhow!("duplicate model preset id '{}'", model.id));
        }
        if !provider_keys.contains(model.provider_key.as_str()) {
            return Err(anyhow!(
                "model preset '{}' references missing provider key '{}'",
                model.id,
                model.provider_key
            ));
        }
        if !matches!(model.role.as_str(), "stt" | "formatting") {
            return Err(anyhow!(
                "model preset '{}' has invalid role '{}'",
                model.id,
                model.role
            ));
        }
        validate_json(
            &model.config_schema,
            &model.config,
            &format!("model preset '{}' config", model.id),
        )?;
    }

    Ok(())
}

fn validate_json(schema: &Value, value: &Value, label: &str) -> Result<()> {
    let compiled = JSONSchema::compile(schema).map_err(|err| anyhow!("compiling {label} schema: {err}"))?;
    if let Err(errors) = compiled.validate(value) {
        let messages = errors
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(anyhow!("invalid {label}: {messages}"));
    }
    Ok(())
}

fn parse_json_column(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<Value> {
    let raw: String = row.try_get(column).with_context(|| format!("reading {column}"))?;
    serde_json::from_str(&raw).with_context(|| format!("parsing {column}"))
}

fn truncate_for_log(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...[truncated {} bytes]", &value[..end], value.len() - end)
}

async fn read_json_response(response: reqwest::Response, context: &ProviderRequestLogContext) -> Result<Value> {
    let status = response.status();
    let headers = headers_to_json(response.headers());
    let body = response.text().await.context("reading provider response body")?;
    let body_bytes = body.len();
    let raw_response = raw_provider_response(context, status, headers, body_bytes, &body);
    if !status.is_success() {
        log::warn!("provider raw response {}", truncate_for_log(&raw_response.to_string(), PROVIDER_BODY_PREVIEW_LIMIT));
        return Err(anyhow!(
            "provider '{}' operation '{}' returned {} raw_response={}",
            context.provider_id,
            context.operation,
            status,
            truncate_for_log(&raw_response.to_string(), PROVIDER_BODY_PREVIEW_LIMIT)
        ));
    }
    log::debug!("provider raw response {}", truncate_for_log(&raw_response.to_string(), PROVIDER_BODY_PREVIEW_LIMIT));
    serde_json::from_str(&body).with_context(|| {
        format!(
            "parsing provider response json raw_response={}",
            truncate_for_log(&raw_response.to_string(), PROVIDER_BODY_PREVIEW_LIMIT)
        )
    })
}

fn log_raw_provider_request(context: &ProviderRequestLogContext, request: Value) {
    let raw_request = json!({
        "operation": context.operation,
        "provider": context.provider_id,
        "user_model_id": context.user_model_id,
        "model_id": context.model_id,
        "external_model": context.external_model_id,
        "request": request,
    });
    log::debug!("provider raw request {}", truncate_for_log(&raw_request.to_string(), PROVIDER_BODY_PREVIEW_LIMIT));
}

fn raw_provider_response(
    context: &ProviderRequestLogContext,
    status: reqwest::StatusCode,
    headers: Value,
    body_bytes: usize,
    body: &str,
) -> Value {
    json!({
        "operation": context.operation,
        "provider": context.provider_id,
        "user_model_id": context.user_model_id,
        "model_id": context.model_id,
        "external_model": context.external_model_id,
        "elapsed_ms": context.elapsed_ms(),
        "response": {
            "status": status.as_u16(),
            "status_text": status.canonical_reason().unwrap_or(""),
            "headers": headers,
            "body_bytes": body_bytes,
            "body": body
        }
    })
}

fn provider_request_headers_for_log(auth: &AuthConfig, content_type: &str) -> Value {
    let authorization = match auth.kind.as_str() {
        "bearer_api_key" if auth.api_key.as_deref().map(str::trim).is_some_and(|value| !value.is_empty()) => {
            Value::String("Bearer [configured]".to_string())
        }
        "bearer_api_key" => Value::String("Bearer [missing]".to_string()),
        _ => Value::Null,
    };
    json!({
        "authorization": authorization,
        "content-type": content_type
    })
}

fn headers_to_json(headers: &reqwest::header::HeaderMap) -> Value {
    let mut out = Map::new();
    for (name, value) in headers {
        out.insert(
            name.as_str().to_string(),
            Value::String(value.to_str().unwrap_or("<non-utf8>").to_string()),
        );
    }
    Value::Object(out)
}

fn apply_auth(request: reqwest::RequestBuilder, auth: &AuthConfig) -> Result<reqwest::RequestBuilder> {
    match auth.kind.as_str() {
        "none" => Ok(request),
        "bearer_api_key" => {
            let api_key = sanitized_bearer_api_key(auth.api_key.as_deref())?;
            Ok(request.bearer_auth(api_key))
        }
        other => Err(anyhow!("unsupported provider auth type '{other}'")),
    }
}

fn sanitized_bearer_api_key(api_key: Option<&str>) -> Result<&str> {
    let api_key = api_key.map(str::trim).filter(|value| !value.is_empty()).ok_or_else(|| {
        anyhow!("provider api key is missing")
    })?;
    if api_key.chars().any(char::is_whitespace) {
        return Err(anyhow!(
            "provider api key is invalid: keys must not contain spaces, newlines, or pasted logs"
        ));
    }
    Ok(api_key)
}

fn validate_runtime_provider_auth(config: &RuntimeProviderConfig) -> Result<()> {
    if config.auth.kind == "bearer_api_key" {
        sanitized_bearer_api_key(config.auth.api_key.as_deref())?;
    }
    Ok(())
}

fn extract_text<'a>(payload: &'a Value, paths: &[&str]) -> Result<&'a str> {
    for path in paths {
        if let Some(text) = payload.pointer(path).and_then(Value::as_str) {
            return Ok(text);
        }
    }
    Err(anyhow!("provider response did not include text at paths {:?}: {}", paths, payload))
}

fn read_audio_file(wav_file: &Path, max_audio_bytes: u64) -> Result<Vec<u8>> {
    let metadata = std::fs::metadata(wav_file)
        .with_context(|| format!("reading audio file metadata: {}", wav_file.display()))?;
    if metadata.len() == 0 {
        return Err(anyhow!("audio file is empty: {}", wav_file.display()));
    }
    if metadata.len() > max_audio_bytes {
        return Err(anyhow!(
            "audio file too large: {} bytes exceeds max_audio_bytes={} for {}",
            metadata.len(),
            max_audio_bytes,
            wav_file.display()
        ));
    }
    std::fs::read(wav_file).with_context(|| format!("reading audio file: {}", wav_file.display()))
}

fn encode_base64(bytes: &[u8]) -> String {
    let encoded_len = bytes.len().saturating_add(2) / 3 * 4;
    let mut encoded = String::with_capacity(encoded_len);
    STANDARD.encode_string(bytes, &mut encoded);
    encoded
}

fn join_url(base_url: &str, endpoint: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        endpoint.trim_start_matches('/')
    )
}

fn insert_optional_string(payload: &mut Value, key: &str, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    if let Some(object) = payload.as_object_mut() {
        object.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn insert_parameters(payload: &mut Value, parameters: &Map<String, Value>) {
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    for (key, value) in parameters {
        object.insert(key.clone(), value.clone());
    }
}

async fn load_config_json(pool: &SqlitePool, table: &str, id: &str) -> Result<Value> {
    let query = match table {
        "providers" => "SELECT config_json FROM providers WHERE id = $1",
        "models" => "SELECT config_json FROM models WHERE id = $1",
        other => return Err(anyhow!("unsupported config table '{other}'")),
    };
    let row = sqlx::query(query)
        .bind(id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("loading {table} config '{id}'"))?;
    parse_json_column(&row, "config_json")
}

async fn save_config_json(pool: &SqlitePool, table: &str, id: &str, config: &Value) -> Result<()> {
    let query = match table {
        "providers" => "UPDATE providers SET config_json = $1, updated_at = $2 WHERE id = $3",
        "models" => "UPDATE models SET config_json = $1, updated_at = $2 WHERE id = $3",
        other => return Err(anyhow!("unsupported config table '{other}'")),
    };
    sqlx::query(query)
        .bind(serde_json::to_string(config).context("serializing config json")?)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await
        .with_context(|| format!("saving {table} config '{id}'"))?;
    Ok(())
}

async fn merge_config_defaults(pool: &SqlitePool, table: &str, id: &str, defaults: &Value) -> Result<()> {
    let mut current = load_config_json(pool, table, id).await?;
    if merge_missing_values(&mut current, defaults) {
        save_config_json(pool, table, id, &current).await?;
    }
    Ok(())
}

fn merge_missing_values(current: &mut Value, defaults: &Value) -> bool {
    match (current, defaults) {
        (Value::Object(current), Value::Object(defaults)) => {
            let mut changed = false;
            for (key, default_value) in defaults {
                match current.get_mut(key) {
                    Some(current_value) => {
                        changed |= merge_missing_values(current_value, default_value);
                    }
                    None => {
                        current.insert(key.clone(), default_value.clone());
                        changed = true;
                    }
                }
            }
            changed
        }
        _ => false,
    }
}

#[derive(Debug, Serialize)]
pub struct RoleModelSettings {
    pub role: String,
    pub model_id: Option<String>,
    pub providers: Vec<ProviderSettingsView>,
    pub models: Vec<CatalogModelView>,
    pub user_models: Vec<UserModelView>,
}

#[derive(Debug, Serialize)]
pub struct ProviderSettingsView {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub config: Value,
    pub override_config: Value,
    pub effective_config: Value,
    pub config_schema: Value,
}

#[derive(Debug, Serialize)]
pub struct CatalogModelView {
    pub id: String,
    pub provider_id: String,
    pub role: String,
    pub external_model_id: String,
    pub display_name: String,
    pub config: Value,
    pub config_schema: Value,
}

#[derive(Debug, Serialize)]
pub struct UserModelView {
    pub id: String,
    pub role: String,
    pub provider_id: String,
    pub provider_name: String,
    pub provider_kind: String,
    pub provider_config: Value,
    pub provider_override_config: Value,
    pub provider_effective_config: Value,
    pub provider_config_schema: Value,
    pub model_id: String,
    pub external_model_id: String,
    pub model_display_name: String,
    pub config: Value,
    pub override_config: Value,
    pub effective_config: Value,
    pub config_schema: Value,
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
pub struct ProviderModelOption {
    pub id: String,
    pub name: String,
}

async fn operation_template_for_role(
    pool: &SqlitePool,
    provider_id: &str,
    role: &str,
) -> Result<(Value, Value, Value, Value)> {
    if let Some(template) = operation_template_from_presets(pool, provider_id, role).await? {
        return Ok(template);
    }

    let row = sqlx::query(
        "SELECT config_json, config_schema_json, adapter_config_json, adapter_config_schema_json
         FROM models
         WHERE provider_id = $1 AND role = $2
         ORDER BY is_preset DESC, updated_at DESC
         LIMIT 1",
    )
    .bind(provider_id)
    .bind(role)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("loading model template for provider '{provider_id}' role '{role}'"))?;

    if let Some(row) = row {
        return Ok((
            parse_json_column(&row, "config_json")?,
            parse_json_column(&row, "config_schema_json")?,
            parse_json_column(&row, "adapter_config_json")?,
            parse_json_column(&row, "adapter_config_schema_json")?,
        ));
    }

    match role {
        "stt" | "formatting" => Err(anyhow!(
            "provider '{}' is missing operation template for role '{}'",
            provider_id,
            role
        )),
        _ => Err(anyhow!("invalid model role '{role}'")),
    }
}

async fn operation_template_from_presets(
    pool: &SqlitePool,
    provider_id: &str,
    role: &str,
) -> Result<Option<(Value, Value, Value, Value)>> {
    let row = sqlx::query("SELECT key FROM providers WHERE id = $1 LIMIT 1")
        .bind(provider_id)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("loading provider key for '{provider_id}'"))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let provider_key: String = row.try_get("key").context("reading provider key")?;
    let presets = load_provider_presets()?;
    let Some(provider) = presets.providers.iter().find(|provider| provider.key == provider_key) else {
        return Ok(None);
    };
    let Some(template) = provider.operation_templates.get(role) else {
        return Ok(None);
    };
    Ok(Some((
        template.request_config.clone(),
        template.request_config_schema.clone(),
        template.adapter_config.clone(),
        template.adapter_config_schema.clone(),
    )))
}

async fn fetch_configured_models(
    client: &Client,
    provider_id: &str,
    config: &RuntimeProviderConfig,
    raw_config: &Value,
    role: &str,
    query_params: &[(String, String)],
) -> Result<Vec<ProviderModelOption>> {
    let response_config: ModelFetchResponseConfig = serde_json::from_value(
        raw_config
            .pointer("/model_fetch/response")
            .cloned()
            .ok_or_else(|| anyhow!("provider '{provider_id}' missing model_fetch.response config"))?,
    )
    .with_context(|| format!("parsing provider '{provider_id}' model_fetch.response config"))?;
    let endpoint = config
        .endpoints
        .get("models")
        .ok_or_else(|| anyhow!("provider '{provider_id}' missing models endpoint"))?;
    let url = join_url(&config.base_url, endpoint);
    let strategy = raw_config
        .pointer("/model_fetch/strategy")
        .and_then(Value::as_str)
        .unwrap_or("manual");
    let context = ProviderRequestLogContext::new("model_fetch", provider_id, &url);
    log_raw_provider_request(
        &context,
        json!({
            "method": "GET",
            "url": &url,
            "headers": provider_request_headers_for_log(&config.auth, "application/json"),
            "query": query_params,
            "body": null,
            "model_fetch": {
                "strategy": strategy,
                "items_path": &response_config.items_path,
                "id_path": &response_config.id_path,
                "name_path": &response_config.name_path,
                "fallback_name_path": &response_config.fallback_name_path
            }
        }),
    );
    let response = apply_auth(client.get(&url).query(query_params), &config.auth)?
        .send()
        .await
        .with_context(|| format!("fetching models from provider '{provider_id}'"))?;
    let payload = read_json_response(response, &context).await?;
    let models = payload
        .pointer(&response_config.items_path)
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow!(
                "provider '{provider_id}' models response missing array at {}",
                response_config.items_path
            )
        })?;
    let role_filter = response_config.filters_by_role.get(role);
    let mut options = Vec::new();
    let mut extracted_sample = Vec::new();
    let mut filtered_sample = Vec::new();
    let mut missing_id_count = 0usize;

    for model in models {
        let Some(id) = model.pointer(&response_config.id_path).and_then(Value::as_str) else {
            missing_id_count += 1;
            continue;
        };
        let name = model
            .pointer(&response_config.name_path)
            .and_then(Value::as_str)
            .or_else(|| {
                response_config
                    .fallback_name_path
                    .as_deref()
                    .and_then(|path| model.pointer(path).and_then(Value::as_str))
            })
            .unwrap_or(id);
        if let Some(reason) = model_role_filter_exclusion_reason(id, name, role_filter) {
            if filtered_sample.len() < PROVIDER_MODEL_SAMPLE_LIMIT {
                filtered_sample.push(format!("{id} reason={reason}"));
            }
            continue;
        }
        if extracted_sample.len() < PROVIDER_MODEL_SAMPLE_LIMIT {
            extracted_sample.push(format!("{id} => {name}"));
        }
        options.push(ProviderModelOption {
            id: id.to_string(),
            name: name.to_string(),
        });
    }

    log::debug!(
        "provider model fetch parsed provider={} role={} raw_count={} returned_count={} filtered_count={} missing_id_count={} sample={:?} filtered_sample={:?}",
        provider_id,
        role,
        models.len(),
        options.len(),
        models.len().saturating_sub(options.len()).saturating_sub(missing_id_count),
        missing_id_count,
        extracted_sample,
        filtered_sample
    );
    Ok(options)
}

fn model_fetch_query_params(config: &Value, role: &str) -> Vec<(String, String)> {
    config
        .pointer(&format!("/model_fetch/query_by_role/{role}"))
        .and_then(Value::as_object)
        .map(|params| {
            params
                .iter()
                .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn model_role_filter_exclusion_reason(id: &str, name: &str, filter: Option<&ModelFetchRoleFilter>) -> Option<&'static str> {
    let Some(filter) = filter else {
        return None;
    };
    let id = id.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    let has_include = !filter.include_exact.is_empty()
        || !filter.include_prefix.is_empty()
        || !filter.include_contains.is_empty();
    let included = !has_include
        || filter.include_exact.iter().any(|value| id == value.to_ascii_lowercase())
        || filter
            .include_prefix
            .iter()
            .any(|value| id.starts_with(&value.to_ascii_lowercase()))
        || filter
            .include_contains
            .iter()
            .any(|value| contains_model_token(&id, &name, value));
    if !included {
        return Some("no_include_rule_matched");
    }
    let excluded = filter.exclude_exact.iter().any(|value| id == value.to_ascii_lowercase())
        || filter
            .exclude_prefix
            .iter()
            .any(|value| id.starts_with(&value.to_ascii_lowercase()))
        || filter
            .exclude_contains
            .iter()
            .any(|value| contains_model_token(&id, &name, value));
    if excluded {
        Some("exclude_rule_matched")
    } else {
        None
    }
}

fn contains_model_token(id: &str, name: &str, value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    id.contains(&value) || name.contains(&value)
}


fn ensure_provider_supports_role(config: &Value, role: &str, provider_id: &str) -> Result<()> {
    if provider_config_supports_role(config, role) {
        Ok(())
    } else {
        Err(anyhow!("provider '{provider_id}' does not support role '{role}'"))
    }
}

fn provider_config_supports_role(config: &Value, role: &str) -> bool {
    config
        .pointer("/capabilities/roles")
        .and_then(Value::as_array)
        .is_some_and(|roles| roles.iter().any(|value| value.as_str() == Some(role)))
}

fn validate_role(role: &str) -> Result<()> {
    if matches!(role, "stt" | "formatting") {
        Ok(())
    } else {
        Err(anyhow!("invalid model role '{role}'"))
    }
}

fn validate_override_object(value: &Value) -> Result<()> {
    if value.is_object() {
        Ok(())
    } else {
        Err(anyhow!("config override must be a JSON object"))
    }
}

fn merge_config_values(base: &mut Value, override_value: &Value) {
    match (base, override_value) {
        (Value::Object(base), Value::Object(override_object)) => {
            for (key, value) in override_object {
                match base.get_mut(key) {
                    Some(base_value) => merge_config_values(base_value, value),
                    None => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, override_value) => {
            *base = override_value.clone();
        }
    }
}
