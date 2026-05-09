use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelHealthStatus {
    Healthy,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelHealthView {
    pub id: String,
    pub role: String,
    pub display_name: String,
    pub provider_name: String,
    pub is_active: bool,
    pub health: ModelHealthStatus,
}

#[derive(Clone)]
pub struct ModelHealthCache {
    inner: Arc<RwLock<Vec<ModelHealthView>>>,
}

struct ModelHealthTarget {
    id: String,
    role: String,
    display_name: String,
    provider_name: String,
    external_model_id: String,
    provider_config: Value,
    provider_override: Value,
    is_active: bool,
}

impl ModelHealthCache {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn snapshot(&self) -> Vec<ModelHealthView> {
        self.inner
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub fn clone_cache(&self) -> Self {
        self.clone()
    }

    pub fn role_snapshot(&self, role: &str) -> Vec<ModelHealthView> {
        self.snapshot()
            .into_iter()
            .filter(|model| model.role == role)
            .collect()
    }

    pub fn record_model_health_outcome(
        &self,
        role: &str,
        user_model_id: &str,
        health: ModelHealthStatus,
    ) -> bool {
        let Ok(mut guard) = self.inner.write() else {
            return false;
        };
        let Some(model) = guard
            .iter_mut()
            .find(|model| model.role == role && model.id == user_model_id)
        else {
            return false;
        };
        model.health = health;
        true
    }

    pub async fn sync_metadata(&self, pool: &SqlitePool) -> Result<()> {
        let targets = load_model_health_targets(pool).await?;
        self.replace_from_targets(targets, None);
        Ok(())
    }

    pub async fn refresh_all(&self, pool: &SqlitePool) -> Result<()> {
        let targets = load_model_health_targets(pool).await?;
        let client = Client::new();
        let mut views = Vec::with_capacity(targets.len());
        for target in targets {
            let health = check_target_health(&client, &target)
                .await
                .unwrap_or(ModelHealthStatus::Unhealthy);
            views.push(view_from_target(target, health));
        }
        if let Ok(mut guard) = self.inner.write() {
            *guard = views;
        }
        Ok(())
    }

    pub async fn refresh_model(
        &self,
        pool: &SqlitePool,
        role: &str,
        user_model_id: &str,
    ) -> Result<ModelHealthStatus> {
        let Some(target) = load_model_health_target(pool, role, user_model_id).await? else {
            self.sync_metadata(pool).await?;
            return Err(anyhow!("model item '{user_model_id}' was not found for role '{role}'"));
        };
        let health = check_target_health(&Client::new(), &target)
            .await
            .unwrap_or(ModelHealthStatus::Unhealthy);
        self.upsert_view(view_from_target(target, health));
        Ok(health)
    }

    fn replace_from_targets(&self, targets: Vec<ModelHealthTarget>, explicit_health: Option<ModelHealthStatus>) {
        let previous = self
            .inner
            .read()
            .map(|guard| {
                guard
                    .iter()
                    .map(|model| (model.id.clone(), model.health))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        let views = targets
            .into_iter()
            .map(|target| {
                let health = explicit_health
                    .or_else(|| previous.get(&target.id).copied())
                    .unwrap_or(ModelHealthStatus::Unknown);
                view_from_target(target, health)
            })
            .collect::<Vec<_>>();
        if let Ok(mut guard) = self.inner.write() {
            *guard = views;
        }
    }

    fn upsert_view(&self, view: ModelHealthView) {
        let Ok(mut guard) = self.inner.write() else {
            return;
        };
        if let Some(existing) = guard
            .iter_mut()
            .find(|model| model.role == view.role && model.id == view.id)
        {
            *existing = view;
        } else {
            guard.push(view);
        }
    }
}

impl Default for ModelHealthCache {
    fn default() -> Self {
        Self::new()
    }
}

fn view_from_target(target: ModelHealthTarget, health: ModelHealthStatus) -> ModelHealthView {
    ModelHealthView {
        id: target.id,
        role: target.role,
        display_name: target.display_name,
        provider_name: target.provider_name,
        is_active: target.is_active,
        health,
    }
}

async fn load_model_health_target(
    pool: &SqlitePool,
    role: &str,
    user_model_id: &str,
) -> Result<Option<ModelHealthTarget>> {
    let row = sqlx::query(
        "SELECT
            um.id AS id,
            um.role AS role,
            um.provider_config_override_json AS provider_override_json,
            um.is_active AS is_active,
            p.name AS provider_name,
            p.config_json AS provider_config_json,
            m.display_name AS display_name,
            m.external_model_id AS external_model_id
         FROM user_models um
         JOIN providers p ON p.id = um.provider_id
         JOIN models m ON m.id = um.model_id
         WHERE um.role = $1 AND um.id = $2
         LIMIT 1",
    )
    .bind(role)
    .bind(user_model_id)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("loading health target '{user_model_id}'"))?;
    row.map(|row| target_from_row(&row)).transpose()
}

async fn load_model_health_targets(pool: &SqlitePool) -> Result<Vec<ModelHealthTarget>> {
    let rows = sqlx::query(
        "SELECT
            um.id AS id,
            um.role AS role,
            um.provider_config_override_json AS provider_override_json,
            um.is_active AS is_active,
            p.name AS provider_name,
            p.config_json AS provider_config_json,
            m.display_name AS display_name,
            m.external_model_id AS external_model_id
         FROM user_models um
         JOIN providers p ON p.id = um.provider_id
         JOIN models m ON m.id = um.model_id
         ORDER BY um.role ASC, um.created_at ASC",
    )
    .fetch_all(pool)
    .await
    .context("loading health targets")?;
    rows.iter().map(target_from_row).collect()
}

fn target_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ModelHealthTarget> {
    Ok(ModelHealthTarget {
        id: row.try_get("id").context("reading health target id")?,
        role: row.try_get("role").context("reading health target role")?,
        display_name: row.try_get("display_name").context("reading health target display name")?,
        provider_name: row.try_get("provider_name").context("reading health target provider name")?,
        external_model_id: row.try_get("external_model_id").context("reading health target external model id")?,
        provider_config: parse_json_column(row, "provider_config_json")?,
        provider_override: parse_json_column(row, "provider_override_json")?,
        is_active: row.try_get::<i64, _>("is_active").context("reading health target active flag")? == 1,
    })
}

async fn check_target_health(client: &Client, target: &ModelHealthTarget) -> Result<ModelHealthStatus> {
    let mut config = target.provider_config.clone();
    merge_config_values(&mut config, &target.provider_override);
    let Some(base_url) = config.get("base_url").and_then(Value::as_str) else {
        log::debug!("model health unknown model={} reason=missing_base_url", target.id);
        return Ok(ModelHealthStatus::Unknown);
    };
    let Some(endpoint) = config.pointer("/endpoints/models").and_then(Value::as_str) else {
        log::debug!("model health unknown model={} reason=missing_models_endpoint", target.id);
        return Ok(ModelHealthStatus::Unknown);
    };
    let url = join_url(base_url, endpoint);
    let query_params = model_fetch_query_params(&config, &target.role);
    let mut request = client.get(&url).query(&query_params);
    let auth_type = config.pointer("/auth/type").and_then(Value::as_str).unwrap_or("none");
    log::debug!(
        "model health request model={} provider={} external_model={} url={} query_params={:?} auth_type={}",
        target.id,
        target.provider_name,
        target.external_model_id,
        url,
        query_params,
        auth_type
    );
    match auth_type {
        "none" => {}
        "bearer_api_key" => {
            let Some(api_key) = config.pointer("/auth/api_key").and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty()) else {
                log::debug!("model health unhealthy model={} reason=missing_api_key", target.id);
                return Ok(ModelHealthStatus::Unhealthy);
            };
            request = request.bearer_auth(api_key);
        }
        other => {
            log::debug!("model health unknown model={} reason=unsupported_auth_type auth_type={}", target.id, other);
            return Ok(ModelHealthStatus::Unknown);
        }
    }

    let response = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            log::debug!("model health connection error model={} url={} error={err:?}", target.id, url);
            return Err(err).with_context(|| format!("checking model health '{}'", target.id));
        }
    };
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        log::debug!(
            "model health unhealthy model={} url={} status={} body={}",
            target.id,
            url,
            status,
            body
        );
        return Ok(ModelHealthStatus::Unhealthy);
    }
    let payload = response
        .json::<Value>()
        .await
        .with_context(|| format!("parsing model health response '{}'", target.id))?;
    if response_contains_model(&payload, &target.external_model_id) {
        log::debug!("model health healthy model={} external_model={}", target.id, target.external_model_id);
        Ok(ModelHealthStatus::Healthy)
    } else {
        log::debug!(
            "model health unhealthy model={} reason=model_not_found external_model={}",
            target.id,
            target.external_model_id
        );
        Ok(ModelHealthStatus::Unhealthy)
    }
}

fn response_contains_model(payload: &Value, external_model_id: &str) -> bool {
    payload
        .get("data")
        .and_then(Value::as_array)
        .is_some_and(|models| models.iter().any(|model| model.get("id").and_then(Value::as_str) == Some(external_model_id)))
        || payload
            .get("models")
            .and_then(Value::as_array)
            .is_some_and(|models| models.iter().any(|model| model.get("name").and_then(Value::as_str) == Some(external_model_id)))
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

fn join_url(base: &str, endpoint: &str) -> String {
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        return endpoint.to_string();
    }
    format!("{}/{}", base.trim_end_matches('/'), endpoint.trim_start_matches('/'))
}

fn merge_config_values(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(key) {
                    Some(base_value) => merge_config_values(base_value, value),
                    None => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, overlay) => {
            *base = overlay.clone();
        }
    }
}

fn parse_json_column(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<Value> {
    let raw: String = row.try_get(column).with_context(|| format!("reading {column}"))?;
    serde_json::from_str(&raw).with_context(|| format!("parsing {column}"))
}
