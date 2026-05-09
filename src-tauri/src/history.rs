use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct HistoryStore {
    pool: SqlitePool,
    notifier: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub audio_file_path: Option<String>,
    pub audio_duration_ms: u64,
    pub transcript_text: Option<String>,
    pub transform_text: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryOverview {
    pub transcripts: i64,
    pub words: i64,
    pub minutes: i64,
}

impl HistoryStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            notifier: None,
        }
    }

    pub fn with_notifier(pool: SqlitePool, notifier: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            pool,
            notifier: Some(notifier),
        }
    }

    pub async fn insert(&self, entry: HistoryEntry) -> Result<()> {
        insert_history(&self.pool, entry).await?;
        if let Some(notifier) = &self.notifier {
            notifier();
        }
        Ok(())
    }
}

pub async fn insert_history(pool: &SqlitePool, entry: HistoryEntry) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let word_count = effective_word_count(&entry);
    let duration_ms = i64::try_from(entry.audio_duration_ms).unwrap_or(i64::MAX);
    sqlx::query(
        "INSERT INTO history (
            id,
            audio_file_path,
            audio_duration_ms,
            transcript_text,
            transform_text,
            error_message,
            word_count,
            created_at,
            updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(entry.audio_file_path)
    .bind(duration_ms)
    .bind(empty_to_none(entry.transcript_text))
    .bind(empty_to_none(entry.transform_text))
    .bind(empty_to_none(entry.error_message))
    .bind(word_count)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn overview(pool: &SqlitePool) -> Result<HistoryOverview> {
    let row = sqlx::query(
        "SELECT
            COUNT(CASE WHEN transcript_text IS NOT NULL AND transcript_text != '' THEN 1 END) AS transcripts,
            COALESCE(SUM(word_count), 0) AS words,
            COALESCE(SUM(audio_duration_ms), 0) AS audio_duration_ms
        FROM history",
    )
    .fetch_one(pool)
    .await?;
    let duration_ms: i64 = row.try_get("audio_duration_ms")?;
    Ok(HistoryOverview {
        transcripts: row.try_get("transcripts")?,
        words: row.try_get("words")?,
        minutes: duration_ms / 60_000,
    })
}

fn effective_word_count(entry: &HistoryEntry) -> i64 {
    entry
        .transform_text
        .as_deref()
        .or(entry.transcript_text.as_deref())
        .map(count_words)
        .unwrap_or(0)
}

fn count_words(text: &str) -> i64 {
    text.split_whitespace().filter(|word| !word.is_empty()).count() as i64
}

fn empty_to_none(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(value)
        }
    })
}
