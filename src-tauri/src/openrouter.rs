use crate::config::OpenRouterConfig;
use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use reqwest::Client;
use std::fs;
use std::path::Path;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct OpenRouterClient {
    client: Client,
    cfg: OpenRouterConfig,
}

impl OpenRouterClient {
    pub fn new(cfg: OpenRouterConfig) -> Result<Self> {
        if cfg.api_key.trim().is_empty() {
            return Err(anyhow::anyhow!("provider.openrouter.api_key missing in config"));
        }

        Ok(Self {
            client: Client::new(),
            cfg,
        })
    }

    pub async fn transcribe(&self, wav_file: &Path) -> Result<String> {
        let start = std::time::Instant::now();
        let file_bytes = fs::read(wav_file).context("reading audio file")?;
        if file_bytes.is_empty() {
            return Err(anyhow::anyhow!("audio file is empty: {}", wav_file.display()));
        }

        let model = self.cfg.model.trim();
        if model.is_empty() {
            return Err(anyhow::anyhow!("provider.openrouter.model is empty"));
        }
        let api_key = self.cfg.api_key.trim();
        if api_key.is_empty() {
            return Err(anyhow::anyhow!("provider.openrouter.api_key missing or empty"));
        }
        let file_size = file_bytes.len();
        let base_url = self.cfg.base_url.trim();
        if base_url.is_empty() {
            return Err(anyhow::anyhow!("provider.openrouter.base_url is empty"));
        }
        let endpoint = self.cfg.endpoint.trim();
        if endpoint.is_empty() {
            return Err(anyhow::anyhow!("provider.openrouter.endpoint is empty"));
        }

        let audio = AudioPayload {
            input_audio: InputAudio {
                data: STANDARD.encode(file_bytes),
                format: "wav".to_string(),
            },
            model: model.to_string(),
            prompt: self.cfg.prompt.clone(),
        };

        log::info!(
            "sending {} byte audio file as model {} to {}",
            file_size,
            model,
            format!("{}/{}", base_url.trim_end_matches('/'), endpoint.trim_start_matches('/'))
        );
        log::debug!("transcribe payload preview: {}", audio_payload_preview(&audio));

        let url = format!(
            "{}/{}",
            base_url.trim_end_matches('/'),
            endpoint.trim_start_matches('/'),
        );
        log::debug!("openrouter endpoint {}", url);

        let response = self
            .client
            .post(url)
            .bearer_auth(api_key)
            .json(&audio)
            .send()
            .await
            .context("posting transcription request")?;

        if !response.status().is_success() {
            let status = response.status();
            let request_id = response
                .headers()
                .get("x-request-id")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
                .unwrap_or_else(|| "unknown".to_string());
            let text = response.text().await.unwrap_or_else(|_| String::new());

            if let Ok(err) = serde_json::from_str::<OpenRouterErrorResponse>(&text) {
                log::warn!(
                    "openrouter error [{}] request_id={} message={}",
                    status,
                    request_id,
                    err.message().unwrap_or("unknown")
                );
                return Err(anyhow::anyhow!(
                    "openrouter returned {status}: {} (request_id={request_id})",
                    err.message().unwrap_or(&text)
                ));
            }

            if status == reqwest::StatusCode::BAD_REQUEST && text.contains("No number after minus sign") {
                log::warn!(
                    "openrouter returned json parse error while parsing request body; request_id={request_id}"
                );
            }
            return Err(anyhow::anyhow!(
                "openrouter returned {status} (request_id={request_id}): {}",
                text
            ));
        }

        let body = response.text().await.context("reading openrouter response body")?;
        let payload: OpenRouterResponse = serde_json::from_str(&body).context("parsing openrouter response")?;
        let text = payload
            .text
            .as_deref()
            .or_else(|| payload.choices.first().and_then(|choice| choice.text.as_deref()))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                log::warn!("openrouter response did not include text: {body}");
                anyhow::anyhow!("openrouter response did not include text")
            })?;
        log::info!("transcription completed in {:?}", start.elapsed());
        log::debug!("transcription completed; text_len={}", text.len());
        Ok(text)
    }
}

#[derive(Deserialize)]
struct OpenRouterResponse {
    text: Option<String>,
    #[serde(default)]
    choices: Vec<OpenRouterChoice>,
}

#[derive(Deserialize)]
struct OpenRouterChoice {
    text: Option<String>,
}

#[derive(Deserialize)]
struct OpenRouterErrorResponse {
    error: OpenRouterErrorPayload,
}

#[derive(Deserialize)]
struct OpenRouterErrorPayload {
    message: Option<String>,
}

#[derive(Debug, Serialize)]
struct AudioPayload {
    input_audio: InputAudio,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
}

#[derive(Debug, Serialize)]
struct InputAudio {
    data: String,
    format: String,
}

fn audio_payload_preview(audio: &AudioPayload) -> String {
    format!(
        "{{\"model\":\"{}\",\"input_audio\":{{\"format\":\"{}\",\"data\":\"<{} bytes>\"}}}}",
        audio.model,
        audio.input_audio.format,
        audio.input_audio.data.len()
    )
}

impl OpenRouterErrorResponse {
    fn message(&self) -> Option<&str> {
        self.error.message.as_deref()
    }
}
