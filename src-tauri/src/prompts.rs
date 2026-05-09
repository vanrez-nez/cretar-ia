use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::collections::HashSet;
use uuid::Uuid;

const PROMPT_PRESETS: &str = include_str!("../prompts/presets.json");

#[derive(Debug, Deserialize)]
struct PromptPresets {
    schema_version: u32,
    prompts: Vec<PromptPreset>,
}

#[derive(Debug, Deserialize)]
struct PromptPreset {
    id: String,
    key: String,
    name: String,
    description: String,
    template: String,
    is_active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptView {
    pub id: String,
    pub key: Option<String>,
    pub name: String,
    pub description: String,
    pub template: String,
    pub is_active: bool,
    pub is_preset: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn seed_prompt_presets(pool: &SqlitePool) -> Result<()> {
    let presets: PromptPresets =
        serde_json::from_str(PROMPT_PRESETS).context("parsing prompt presets")?;
    validate_prompt_presets(&presets)?;

    let now = chrono::Utc::now().to_rfc3339();
    for prompt in &presets.prompts {
        sqlx::query(
            "INSERT INTO prompts (
                id, key, name, description, template, is_active, is_preset, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?)
             ON CONFLICT(key) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                template = excluded.template,
                is_preset = 1,
                updated_at = excluded.updated_at",
        )
        .bind(&prompt.id)
        .bind(&prompt.key)
        .bind(&prompt.name)
        .bind(&prompt.description)
        .bind(&prompt.template)
        .bind(if prompt.is_active { 1 } else { 0 })
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .with_context(|| format!("seeding prompt preset '{}'", prompt.key))?;
    }

    let active_count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM prompts WHERE is_active = 1")
        .fetch_one(pool)
        .await
        .context("counting active prompts after seed")?
        .try_get("count")
        .context("reading active prompt count after seed")?;
    if active_count == 0 {
        select_first_prompt(pool).await?;
    }

    Ok(())
}

pub async fn list_prompts(pool: &SqlitePool) -> Result<Vec<PromptView>> {
    let rows = sqlx::query(
        "SELECT id, key, name, description, template, is_active, is_preset, created_at, updated_at
         FROM prompts
         ORDER BY created_at ASC, id ASC",
    )
    .fetch_all(pool)
    .await
    .context("loading prompts")?;

    rows.into_iter().map(prompt_from_row).collect()
}

pub async fn active_prompt(pool: &SqlitePool) -> Result<Option<PromptView>> {
    let row = sqlx::query(
        "SELECT id, key, name, description, template, is_active, is_preset, created_at, updated_at
         FROM prompts
         WHERE is_active = 1
         ORDER BY created_at ASC, id ASC
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("loading active prompt")?;

    row.map(prompt_from_row).transpose()
}

pub async fn save_prompt(
    pool: &SqlitePool,
    prompt_id: Option<String>,
    name: String,
    description: String,
    template: String,
) -> Result<String> {
    let name = name.trim();
    let description = description.trim();
    let template = template.trim();

    if name.is_empty() {
        return Err(anyhow!("prompt name is required"));
    }
    if description.is_empty() {
        return Err(anyhow!("prompt description is required"));
    }
    if template.is_empty() {
        return Err(anyhow!("prompt template is required"));
    }

    let updated_at = chrono::Utc::now().to_rfc3339();

    if let Some(prompt_id) = prompt_id {
        let row = sqlx::query("SELECT is_preset FROM prompts WHERE id = ?")
            .bind(&prompt_id)
            .fetch_optional(pool)
            .await
            .with_context(|| format!("loading prompt {prompt_id}"))?
            .ok_or_else(|| anyhow!("prompt not found"))?;
        let is_preset: i64 = row.try_get("is_preset").context("reading prompt preset flag")?;
        if is_preset != 0 {
            return Err(anyhow!("preset prompts cannot be edited"));
        }

        sqlx::query(
            "UPDATE prompts
             SET name = ?, description = ?, template = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(name)
        .bind(description)
        .bind(template)
        .bind(&updated_at)
        .bind(&prompt_id)
        .execute(pool)
        .await
        .with_context(|| format!("updating prompt {prompt_id}"))?;

        return Ok(prompt_id);
    }

    let id = Uuid::new_v4().to_string();
    let active_count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM prompts WHERE is_active = 1")
        .fetch_one(pool)
        .await
        .context("counting active prompts")?
        .try_get("count")
        .context("reading active prompt count")?;
    let is_active = if active_count == 0 { 1 } else { 0 };

    sqlx::query(
        "INSERT INTO prompts (
            id,
            name,
            description,
            template,
            is_active,
            is_preset,
            created_at,
            updated_at
         ) VALUES (?, ?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(description)
    .bind(template)
    .bind(is_active)
    .bind(&updated_at)
    .bind(&updated_at)
    .execute(pool)
    .await
    .with_context(|| format!("creating prompt {id}"))?;

    Ok(id)
}

pub async fn delete_prompt(pool: &SqlitePool, prompt_id: &str) -> Result<()> {
    let row = sqlx::query("SELECT is_active, is_preset FROM prompts WHERE id = ?")
        .bind(prompt_id)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("loading prompt {prompt_id}"))?
        .ok_or_else(|| anyhow!("prompt not found"))?;
    let was_active: i64 = row.try_get("is_active").context("reading prompt active flag")?;
    let is_preset: i64 = row.try_get("is_preset").context("reading prompt preset flag")?;
    if is_preset != 0 {
        return Err(anyhow!("preset prompts cannot be deleted"));
    }

    sqlx::query("DELETE FROM prompts WHERE id = ?")
        .bind(prompt_id)
        .execute(pool)
        .await
        .with_context(|| format!("deleting prompt {prompt_id}"))?;

    if was_active != 0 {
        select_first_prompt(pool).await?;
    }

    Ok(())
}

pub async fn select_prompt(pool: &SqlitePool, prompt_id: &str) -> Result<()> {
    let exists: i64 = sqlx::query("SELECT COUNT(*) AS count FROM prompts WHERE id = ?")
        .bind(prompt_id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("loading prompt {prompt_id}"))?
        .try_get("count")
        .context("reading prompt count")?;
    if exists == 0 {
        return Err(anyhow!("prompt not found"));
    }

    let updated_at = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE prompts SET is_active = 0, updated_at = ? WHERE is_active = 1")
        .bind(&updated_at)
        .execute(pool)
        .await
        .context("clearing active prompt")?;
    sqlx::query("UPDATE prompts SET is_active = 1, updated_at = ? WHERE id = ?")
        .bind(&updated_at)
        .bind(prompt_id)
        .execute(pool)
        .await
        .with_context(|| format!("selecting prompt {prompt_id}"))?;

    Ok(())
}

async fn select_first_prompt(pool: &SqlitePool) -> Result<()> {
    let next_id = sqlx::query("SELECT id FROM prompts ORDER BY created_at ASC, id ASC LIMIT 1")
        .fetch_optional(pool)
        .await
        .context("loading fallback prompt")?
        .map(|row| row.try_get::<String, _>("id"))
        .transpose()
        .context("reading fallback prompt id")?;

    if let Some(next_id) = next_id {
        select_prompt(pool, &next_id).await?;
    }

    Ok(())
}

fn prompt_from_row(row: sqlx::sqlite::SqliteRow) -> Result<PromptView> {
    let is_active: i64 = row.try_get("is_active").context("reading prompt active flag")?;
    let is_preset: i64 = row.try_get("is_preset").context("reading prompt preset flag")?;

    Ok(PromptView {
        id: row.try_get("id").context("reading prompt id")?,
        key: row.try_get("key").context("reading prompt key")?,
        name: row.try_get("name").context("reading prompt name")?,
        description: row
            .try_get("description")
            .context("reading prompt description")?,
        template: row.try_get("template").context("reading prompt template")?,
        is_active: is_active != 0,
        is_preset: is_preset != 0,
        created_at: row.try_get("created_at").context("reading prompt created_at")?,
        updated_at: row.try_get("updated_at").context("reading prompt updated_at")?,
    })
}

fn validate_prompt_presets(presets: &PromptPresets) -> Result<()> {
    if presets.schema_version != 1 {
        return Err(anyhow!(
            "unsupported prompt presets schema_version {}",
            presets.schema_version
        ));
    }

    let mut ids = HashSet::new();
    let mut keys = HashSet::new();
    for prompt in &presets.prompts {
        Uuid::parse_str(&prompt.id)
            .with_context(|| format!("prompt preset '{}' id must be a UUID", prompt.key))?;
        if !ids.insert(prompt.id.as_str()) {
            return Err(anyhow!("duplicate prompt preset id '{}'", prompt.id));
        }
        if !keys.insert(prompt.key.as_str()) {
            return Err(anyhow!("duplicate prompt preset key '{}'", prompt.key));
        }
        if prompt.name.trim().is_empty() {
            return Err(anyhow!("prompt preset '{}' name is required", prompt.key));
        }
        if prompt.description.trim().is_empty() {
            return Err(anyhow!("prompt preset '{}' description is required", prompt.key));
        }
        if prompt.template.trim().is_empty() {
            return Err(anyhow!("prompt preset '{}' template is required", prompt.key));
        }
    }

    Ok(())
}
