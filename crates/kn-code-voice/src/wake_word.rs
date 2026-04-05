use crate::audio::AudioRecorder;
use crate::stt::{SpeechToText, SttConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

/// Wake word detection mode.
///
/// - Onnx: Uses OpenWakeWord ONNX models for real-time audio-based detection.
///   Requires .onnx model files in the model directory.
/// - SttFake: Continuously transcribes audio via STT and checks if the
///   transcription starts with a wake word phrase. Simpler, works anywhere
///   STT works (including Telegram voice messages), but higher latency.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WakeWordMode {
    #[default]
    Onnx,
    SttFake,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakeWordConfig {
    pub mode: WakeWordMode,
    pub model_dir: PathBuf,
    pub wake_words: Vec<String>,
    pub confidence_threshold: f32,
    pub detection_cooldown_ms: u64,
    pub chunk_duration_ms: u64,
    pub stt_config: Option<SttConfig>,
}

impl Default for WakeWordConfig {
    fn default() -> Self {
        Self {
            mode: WakeWordMode::default(),
            model_dir: default_model_dir(),
            wake_words: vec!["hey kn code".to_string(), "hey code".to_string()],
            confidence_threshold: 0.5,
            detection_cooldown_ms: 2000,
            chunk_duration_ms: 10,
            stt_config: None,
        }
    }
}

fn default_model_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".kn-code")
        .join("voice")
        .join("models")
}

struct WakeWordModel {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    path: PathBuf,
}

/// Wake word detector supporting both ONNX and STT-based detection.
///
/// ONNX mode: Streams audio through OpenWakeWord models, fires instantly
/// when confidence exceeds threshold. Low latency, requires model files.
///
/// STT Fake mode: Records audio chunks, transcribes them via Whisper,
/// checks if transcription starts with a wake word phrase. Higher latency
/// but works everywhere STT works — including Telegram voice messages where
/// there's no continuous audio stream to run ONNX models on.
pub struct WakeWordDetector {
    config: WakeWordConfig,
    active: Arc<AtomicBool>,
    models: HashMap<String, WakeWordModel>,
}

impl WakeWordDetector {
    pub fn new(config: WakeWordConfig) -> Self {
        Self {
            config,
            active: Arc::new(AtomicBool::new(false)),
            models: HashMap::new(),
        }
    }

    /// Load ONNX wake word models from the model directory.
    /// Only needed for Onnx mode — SttFake mode skips this.
    pub async fn load_models(&mut self) -> anyhow::Result<()> {
        if self.config.mode == WakeWordMode::SttFake {
            tracing::info!("STT fake wake word mode — no ONNX models needed");
            return Ok(());
        }

        for word in &self.config.wake_words {
            let model_path = self
                .config
                .model_dir
                .join(format!("{}.onnx", word.replace(' ', "_")));

            if model_path.exists() {
                self.models.insert(
                    word.clone(),
                    WakeWordModel {
                        name: word.clone(),
                        path: model_path.clone(),
                    },
                );
                tracing::info!(
                    "Loaded wake word model: {} -> {:?}",
                    word,
                    model_path.clone()
                );
            } else {
                tracing::warn!(
                    "Wake word model not found: {} (expected at {:?})",
                    word,
                    model_path
                );
            }
        }
        Ok(())
    }

    /// Start listening for wake words. Returns a channel that fires when detected.
    ///
    /// The detection method depends on config.mode:
    /// - Onnx: Real-time ONNX model inference on audio stream
    /// - SttFake: Continuous STT transcription with wake word phrase matching
    pub async fn start_listening(&self) -> anyhow::Result<mpsc::Receiver<String>> {
        let (tx, rx) = mpsc::channel(10);

        match self.config.mode {
            WakeWordMode::Onnx => self.start_onnx_listening(tx.clone()).await?,
            WakeWordMode::SttFake => self.start_stt_listening(tx.clone()).await?,
        }

        self.active.store(true, Ordering::SeqCst);
        Ok(rx)
    }

    /// ONNX-based wake word detection.
    /// Streams audio through OpenWakeWord models in real-time.
    async fn start_onnx_listening(&self, _tx: mpsc::Sender<String>) -> anyhow::Result<()> {
        anyhow::bail!(
            "ONNX wake word detection is not yet implemented. \
             Use text-based wake word detection instead, or integrate \
             openwakeword-rs for real-time audio inference."
        )
    }

    /// STT-based fake wake word detection.
    ///
    /// Records fixed-duration audio chunks, transcribes them via Whisper,
    /// then checks if the transcription starts with any configured wake word.
    ///
    /// This works everywhere STT works — including Telegram voice messages
    /// where there's no continuous audio stream for ONNX inference.
    /// Trade-off: higher latency (must wait for full STT result) but
    /// zero model files needed.
    async fn start_stt_listening(&self, tx: mpsc::Sender<String>) -> anyhow::Result<()> {
        let config = self.config.clone();
        let active = self.active.clone();
        let wake_words = self.config.wake_words.clone();
        let stt_config = self.config.stt_config.clone().unwrap_or_default();

        tokio::spawn(async move {
            let stt = SpeechToText::new(stt_config);
            let mut last_detection = std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(10))
                .unwrap_or(std::time::Instant::now());

            while active.load(Ordering::SeqCst) {
                // Record a fixed 5-second chunk
                let temp_path =
                    std::env::temp_dir().join(format!("kn-wake-{}.wav", uuid::Uuid::new_v4()));

                let recorder = AudioRecorder::new(crate::audio::AudioConfig {
                    max_recording_secs: 5,
                    silence_threshold: 1.0, // effectively disable silence detection
                    ..Default::default()
                });

                let record_result = recorder.record_until_silence(temp_path.clone()).await;

                // Recorder is dropped here, releasing any non-Send resources
                drop(recorder);

                match record_result {
                    Ok(audio_path) => {
                        let cooldown_elapsed = last_detection.elapsed().as_millis()
                            >= config.detection_cooldown_ms as u128;

                        if !cooldown_elapsed {
                            let _ = tokio::fs::remove_file(&audio_path).await;
                            continue;
                        }

                        match stt.transcribe(&audio_path).await {
                            Ok(result) => {
                                let text = result.text.to_lowercase().trim().to_string();
                                tracing::debug!("STT wake check: '{}'", text);

                                for word in &wake_words {
                                    if text.starts_with(&word.to_lowercase()) {
                                        tracing::info!("STT wake word detected: '{}'", word);
                                        let _ = tx.send(word.clone()).await;
                                        last_detection = std::time::Instant::now();
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::debug!("STT wake transcription failed: {}", e);
                            }
                        }

                        let _ = tokio::fs::remove_file(&audio_path).await;
                    }
                    Err(e) => {
                        tracing::debug!("STT wake recording failed: {}", e);
                    }
                }
            }
        });

        Ok(())
    }

    /// Check if a text transcription contains a wake word phrase.
    /// Used by Telegram and other channels where audio arrives as
    /// complete voice messages rather than a continuous stream.
    pub fn check_text_for_wake_word(&self, text: &str) -> Option<String> {
        let lower = text.to_lowercase().trim().to_string();

        for word in &self.config.wake_words {
            if lower.starts_with(&word.to_lowercase()) {
                return Some(word.clone());
            }
        }

        None
    }

    /// Strip the wake word prefix from text.
    /// "hey kn code build the project" -> "build the project"
    pub fn strip_wake_word(&self, text: &str) -> String {
        let lower = text.to_lowercase();

        for word in &self.config.wake_words {
            if lower.starts_with(&word.to_lowercase()) {
                let char_count = word.chars().count();
                return text
                    .chars()
                    .skip(char_count)
                    .collect::<String>()
                    .trim()
                    .to_string();
            }
        }

        text.to_string()
    }

    pub fn stop_listening(&self) {
        self.active.store(false, Ordering::SeqCst);
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    pub fn configured_words(&self) -> &[String] {
        &self.config.wake_words
    }

    pub fn mode(&self) -> &WakeWordMode {
        &self.config.mode
    }
}
