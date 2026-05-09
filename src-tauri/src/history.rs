use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use std::path::Path;
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryAudioItem {
    pub id: String,
    pub audio_file_path: String,
    pub audio_duration_ms: u64,
    pub transcript_text: Option<String>,
    pub transform_text: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioWaveform {
    pub duration: f64,
    pub peaks: Vec<Vec<f32>>,
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

pub async fn latest_audio(pool: &SqlitePool) -> Result<Option<HistoryAudioItem>> {
    let rows = sqlx::query(
        "SELECT id, audio_file_path, audio_duration_ms, transcript_text, transform_text, created_at
         FROM history
         WHERE audio_file_path IS NOT NULL AND audio_file_path != ''
         ORDER BY created_at DESC, id DESC
         LIMIT 20",
    )
    .fetch_all(pool)
    .await?;

    for row in rows {
        let audio_file_path: String = row.try_get("audio_file_path")?;
        if !Path::new(&audio_file_path).is_file() {
            log::debug!("history audio skipped because file is missing: {audio_file_path}");
            continue;
        }

        let duration_ms: i64 = row.try_get("audio_duration_ms")?;
        return Ok(Some(HistoryAudioItem {
            id: row.try_get("id")?,
            audio_file_path,
            audio_duration_ms: duration_ms.max(0) as u64,
            transcript_text: row.try_get("transcript_text")?,
            transform_text: row.try_get("transform_text")?,
            created_at: row.try_get("created_at")?,
        }));
    }

    Ok(None)
}

pub async fn waveform(pool: &SqlitePool, history_id: &str, samples: Option<u32>) -> Result<AudioWaveform> {
    let row = sqlx::query("SELECT audio_file_path FROM history WHERE id = ?")
        .bind(history_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow!("history row not found"))?;

    let audio_file_path: Option<String> = row.try_get("audio_file_path")?;
    let audio_file_path = audio_file_path
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| anyhow!("history row has no retained audio file"))?;
    let path = Path::new(&audio_file_path);
    if !path.is_file() {
        return Err(anyhow!("audio file is no longer available"));
    }

    read_wav_waveform(path, samples.unwrap_or(1024))
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

fn read_wav_waveform(path: &Path, samples: u32) -> Result<AudioWaveform> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("opening audio file for waveform: {}", path.display()))?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels.max(1));
    let sample_rate = f64::from(spec.sample_rate.max(1));
    let total_samples = reader.duration() as usize;
    let total_frames = (total_samples / channels).max(1);
    let bucket_count = (samples.clamp(64, 2048) as usize).max(1);
    let mut peaks = vec![0.0_f32; bucket_count];
    let duration = total_frames as f64 / sample_rate;

    match spec.sample_format {
        hound::SampleFormat::Float => {
            for (sample_index, sample) in reader.samples::<f32>().enumerate() {
                record_peak(&mut peaks, sample?, sample_index, channels, total_frames);
            }
        }
        hound::SampleFormat::Int => {
            if spec.bits_per_sample <= 16 {
                let max = ((1_i64 << (spec.bits_per_sample.saturating_sub(1) as u32)) - 1).max(1) as f32;
                for (sample_index, sample) in reader.samples::<i16>().enumerate() {
                    record_peak(&mut peaks, sample? as f32 / max, sample_index, channels, total_frames);
                }
            } else {
                let max = ((1_i64 << (spec.bits_per_sample.saturating_sub(1).min(31) as u32)) - 1).max(1) as f32;
                for (sample_index, sample) in reader.samples::<i32>().enumerate() {
                    record_peak(&mut peaks, sample? as f32 / max, sample_index, channels, total_frames);
                }
            }
        }
    }

    Ok(AudioWaveform {
        duration,
        peaks: vec![peaks],
    })
}

fn record_peak(peaks: &mut [f32], sample: f32, sample_index: usize, channels: usize, total_frames: usize) {
    let frame_index = sample_index / channels;
    let bucket = (frame_index.saturating_mul(peaks.len()) / total_frames).min(peaks.len().saturating_sub(1));
    let amplitude = sample.clamp(-1.0, 1.0);
    if amplitude.abs() > peaks[bucket].abs() {
        peaks[bucket] = amplitude;
    }
}
