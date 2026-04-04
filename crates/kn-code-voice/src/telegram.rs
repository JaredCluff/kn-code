use crate::manager::VoiceEvent;
use crate::stt::SpeechToText;
use crate::tts::TextToSpeech;
use crate::wake_word::WakeWordDetector;
use secrecy::{ExposeSecret, Secret};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

/// Telegram voice message from a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramVoiceMessage {
    pub chat_id: i64,
    pub message_id: i64,
    pub user_id: i64,
    pub file_id: String,
    pub duration_secs: u32,
    pub mime_type: Option<String>,
}

/// Telegram bot configuration for voice channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    #[serde(
        serialize_with = "serialize_secret",
        deserialize_with = "deserialize_secret"
    )]
    pub bot_token: Secret<String>,
    pub allowed_user_ids: Vec<i64>,
    pub wake_word_required: bool,
    pub auto_respond: bool,
    pub voice_response: bool,
}

fn serialize_secret<S>(_secret: &Secret<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str("***REDACTED***")
}

fn deserialize_secret<'de, D>(deserializer: D) -> Result<Secret<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(Secret::new(s))
}

/// Telegram voice channel handler.
///
/// Flow:
/// 1. Receive voice message from Telegram Bot API
/// 2. Download OGG file
/// 3. Convert OGG opus → WAV (16kHz mono) using ffmpeg
/// 4. Check for wake word in transcription (STT fake mode)
/// 5. If wake word detected (or not required), process command
/// 6. Send text or voice response back via Telegram
pub struct TelegramVoiceChannel {
    pub config: TelegramConfig,
    detector: WakeWordDetector,
    stt: SpeechToText,
    tts: TextToSpeech,
    event_tx: mpsc::Sender<VoiceEvent>,
    client: reqwest::Client,
}

impl TelegramVoiceChannel {
    pub fn new(
        config: TelegramConfig,
        detector: WakeWordDetector,
        stt: SpeechToText,
        tts: TextToSpeech,
        event_tx: mpsc::Sender<VoiceEvent>,
    ) -> Self {
        Self {
            config,
            detector,
            stt,
            tts,
            event_tx,
            client: reqwest::Client::new(),
        }
    }

    /// Process a voice message received via Telegram.
    ///
    /// This is the main entry point called by the Telegram bot handler
    /// when a voice message arrives.
    pub async fn process_voice_message(
        &self,
        msg: TelegramVoiceMessage,
    ) -> anyhow::Result<VoiceProcessingResult> {
        if !self.is_user_allowed(msg.user_id) {
            tracing::warn!("Voice message from unauthorized user: {}", msg.user_id);
            return Ok(VoiceProcessingResult::Denied {
                user_id: msg.user_id,
            });
        }

        let _ = self
            .event_tx
            .send(VoiceEvent::VoiceMessageReceived {
                source: VoiceSource::Telegram {
                    chat_id: msg.chat_id,
                    user_id: msg.user_id,
                },
            })
            .await;

        // Download the voice message
        let ogg_path = self.download_voice(&msg.file_id).await?;

        // Convert OGG opus → WAV
        let wav_path = self.convert_ogg_to_wav(&ogg_path).await?;

        // Transcribe
        let transcription = self.stt.transcribe(&wav_path).await?;

        let _ = self
            .event_tx
            .send(VoiceEvent::Transcription {
                text: transcription.text.clone(),
            })
            .await;

        // Check for wake word
        let text = if self.config.wake_word_required {
            if let Some(_word) = self.detector.check_text_for_wake_word(&transcription.text) {
                self.detector.strip_wake_word(&transcription.text)
            } else {
                // No wake word — ignore the message
                tracing::debug!("No wake word detected in Telegram voice message");
                let _ = tokio::fs::remove_file(&ogg_path).await;
                let _ = tokio::fs::remove_file(&wav_path).await;
                return Ok(VoiceProcessingResult::NoWakeWord {
                    transcription: transcription.text,
                });
            }
        } else {
            transcription.text.clone()
        };

        // Clean up temp files
        let _ = tokio::fs::remove_file(&ogg_path).await;
        let _ = tokio::fs::remove_file(&wav_path).await;

        // Send the command to kn-code for processing
        let _ = self
            .event_tx
            .send(VoiceEvent::CommandReady {
                text: text.clone(),
                source: VoiceSource::Telegram {
                    chat_id: msg.chat_id,
                    user_id: msg.user_id,
                },
            })
            .await;

        Ok(VoiceProcessingResult::Processed {
            command: text,
            transcription: transcription.text,
            chat_id: msg.chat_id,
        })
    }

    /// Send a text response back to Telegram.
    pub async fn send_text_response(&self, chat_id: i64, text: &str) -> anyhow::Result<()> {
        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            self.config.bot_token.expose_secret()
        );

        let body = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "Markdown",
        });

        self.client.post(&url).json(&body).send().await?;

        Ok(())
    }

    /// Send a voice response back to Telegram (TTS → voice message).
    pub async fn send_voice_response(&self, chat_id: i64, text: &str) -> anyhow::Result<()> {
        if !self.config.voice_response {
            return self.send_text_response(chat_id, text).await;
        }

        // Synthesize speech
        let wav_path = std::env::temp_dir().join(format!("kn-tg-tts-{}.wav", uuid::Uuid::new_v4()));
        self.tts.synthesize_to_file(text, wav_path.clone()).await?;

        // Convert WAV → OGG opus for Telegram
        let ogg_path = wav_path.with_extension("ogg");
        self.convert_wav_to_ogg(&wav_path, &ogg_path).await?;

        // Send as voice message
        let url = format!(
            "https://api.telegram.org/bot{}/sendVoice",
            self.config.bot_token.expose_secret()
        );

        let file_bytes = tokio::fs::read(&ogg_path).await?;
        let part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name("voice.ogg")
            .mime_str("audio/ogg")?;

        let form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("voice", part);

        self.client.post(&url).multipart(form).send().await?;

        // Clean up
        let _ = tokio::fs::remove_file(&wav_path).await;
        let _ = tokio::fs::remove_file(&ogg_path).await;

        Ok(())
    }

    /// Download a voice message from Telegram.
    async fn download_voice(&self, file_id: &str) -> anyhow::Result<PathBuf> {
        // Get file path from Telegram API
        let url = format!(
            "https://api.telegram.org/bot{}/getFile",
            self.config.bot_token.expose_secret()
        );

        let response = self
            .client
            .get(&url)
            .query(&[("file_id", file_id)])
            .send()
            .await?;

        let json: serde_json::Value = response.json().await?;
        let file_path = json["result"]["file_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("No file_path in Telegram response"))?;

        // Download the file
        let download_url = format!(
            "https://api.telegram.org/file/bot{}/{}",
            self.config.bot_token.expose_secret(),
            file_path
        );

        let bytes = self.client.get(&download_url).send().await?.bytes().await?;

        let output_path = std::env::temp_dir().join(format!("kn-tg-{}.ogg", uuid::Uuid::new_v4()));
        tokio::fs::write(&output_path, &bytes).await?;

        Ok(output_path)
    }

    /// Convert OGG opus → WAV (16kHz mono) using ffmpeg.
    async fn convert_ogg_to_wav(&self, ogg_path: &Path) -> anyhow::Result<PathBuf> {
        let wav_path = ogg_path.with_extension("wav");

        let output = tokio::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-i",
                ogg_path
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("Invalid ogg path"))?,
                "-ar",
                "16000",
                "-ac",
                "1",
                "-sample_fmt",
                "s16",
                wav_path
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("Invalid wav path"))?,
            ])
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!(
                "ffmpeg conversion failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(wav_path)
    }

    /// Convert WAV → OGG opus for Telegram voice messages.
    async fn convert_wav_to_ogg(&self, wav_path: &Path, ogg_path: &Path) -> anyhow::Result<()> {
        let output = tokio::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-i",
                wav_path
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("Invalid wav path"))?,
                "-c:a",
                "libopus",
                "-b:a",
                "32k",
                "-application",
                "voip",
                ogg_path
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("Invalid ogg path"))?,
            ])
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!(
                "ffmpeg OGG conversion failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    fn is_user_allowed(&self, user_id: i64) -> bool {
        if self.config.allowed_user_ids.is_empty() {
            tracing::warn!("No allowed_user_ids configured — denying all Telegram access");
            return false;
        }
        self.config.allowed_user_ids.contains(&user_id)
    }
}

/// Result of processing a Telegram voice message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VoiceProcessingResult {
    Processed {
        command: String,
        transcription: String,
        chat_id: i64,
    },
    NoWakeWord {
        transcription: String,
    },
    Denied {
        user_id: i64,
    },
}

/// Source of a voice command.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VoiceSource {
    Microphone,
    Telegram { chat_id: i64, user_id: i64 },
}
