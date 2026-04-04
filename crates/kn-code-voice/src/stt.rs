use secrecy::{ExposeSecret, Secret};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttConfig {
    pub engine: SttEngine,
    pub model: String,
    pub language: Option<String>,
    #[serde(skip_serializing)]
    pub api_key: Option<Secret<String>>,
    pub api_url: Option<String>,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            engine: SttEngine::default(),
            model: "base".to_string(),
            language: Some("en".to_string()),
            api_key: None,
            api_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SttEngine {
    #[default]
    WhisperLocal,
    WhisperApi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionResult {
    pub text: String,
    pub language: String,
    pub segments: Vec<TranscriptionSegment>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub confidence: f32,
}

pub struct SpeechToText {
    config: SttConfig,
}

impl SpeechToText {
    pub fn new(config: SttConfig) -> Self {
        Self { config }
    }

    /// Transcribe an audio file (WAV format, 16kHz mono).
    pub async fn transcribe(&self, audio_path: &Path) -> anyhow::Result<TranscriptionResult> {
        match &self.config.engine {
            SttEngine::WhisperLocal => self.transcribe_local(audio_path).await,
            SttEngine::WhisperApi => self.transcribe_api(audio_path).await,
        }
    }

    async fn transcribe_local(&self, _audio_path: &Path) -> anyhow::Result<TranscriptionResult> {
        // TODO: Integrate whisper.cpp via whisper-rs or whisper-rs bindings
        // whisper.cpp runs locally, no API needed
        // For now, return a placeholder
        tracing::info!("Local Whisper transcription requested (not yet implemented)");
        Ok(TranscriptionResult {
            text: String::new(),
            language: "en".to_string(),
            segments: Vec::new(),
            duration_ms: 0,
        })
    }

    async fn transcribe_api(&self, audio_path: &Path) -> anyhow::Result<TranscriptionResult> {
        let api_key = self
            .config
            .api_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("API key required for Whisper API"))?;

        let api_url = self
            .config
            .api_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1/audio/transcriptions");

        let model = self.config.model.clone();
        let language = self
            .config
            .language
            .clone()
            .unwrap_or_else(|| "en".to_string());

        let client = reqwest::Client::new();
        let file_name = audio_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("audio.wav");

        let file_bytes = tokio::fs::read(audio_path).await?;
        let form = reqwest::multipart::Form::new().text("model", model).part(
            "file",
            reqwest::multipart::Part::bytes(file_bytes)
                .file_name(file_name.to_string())
                .mime_str("audio/wav")?,
        );

        let response = client
            .post(api_url)
            .header(
                "Authorization",
                format!("Bearer {}", api_key.expose_secret()),
            )
            .multipart(form)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Whisper API error ({}): {}", status, body);
        }

        let result: serde_json::Value = response.json().await?;
        let text = result
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(TranscriptionResult {
            text,
            language,
            segments: Vec::new(),
            duration_ms: 0,
        })
    }
}
