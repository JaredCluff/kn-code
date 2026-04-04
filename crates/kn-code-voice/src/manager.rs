use crate::audio::{AudioConfig, AudioRecorder};
use crate::stt::{SpeechToText, SttConfig, TranscriptionResult};
use crate::telegram::{TelegramVoiceChannel, VoiceSource};
use crate::tts::{TextToSpeech, TtsConfig};
use crate::wake_word::{WakeWordConfig, WakeWordDetector};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::sync::mpsc;

/// Voice mode state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceState {
    Idle,
    Listening,
    Recording,
    Transcribing,
    Processing,
    Speaking,
    Error(String),
}

/// Configuration for the full voice pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    pub audio: AudioConfig,
    pub wake_word: WakeWordConfig,
    pub stt: SttConfig,
    pub tts: TtsConfig,
    pub voice_dir: PathBuf,
    pub auto_start: bool,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        let voice_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".kn-code")
            .join("voice");

        Self {
            audio: AudioConfig::default(),
            wake_word: WakeWordConfig::default(),
            stt: SttConfig {
                engine: crate::stt::SttEngine::WhisperLocal,
                model: "base".to_string(),
                language: Some("en".to_string()),
                api_key: None,
                api_url: None,
            },
            tts: TtsConfig::default(),
            voice_dir,
            auto_start: false,
        }
    }
}

/// Voice event emitted by the manager.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VoiceEvent {
    WakeWordDetected { word: String },
    VoiceMessageReceived { source: VoiceSource },
    RecordingStarted,
    RecordingStopped,
    Transcription { text: String },
    CommandReady { text: String, source: VoiceSource },
    Speaking { text: String },
    StateChange { state: VoiceState },
    Error { message: String },
}

/// Manages the full voice pipeline.
///
/// Flow:
/// 1. Idle — listening for wake word
/// 2. Wake word detected — start recording
/// 3. Silence detected — stop recording
/// 4. Transcribe audio → text
/// 5. Text sent to caller (kn-code agent)
/// 6. Agent response → TTS → speak
/// 7. Return to idle
pub struct VoiceManager {
    pub config: VoiceConfig,
    pub state: VoiceState,
    recorder: Option<AudioRecorder>,
    wake_word_detector: Option<WakeWordDetector>,
    stt: Option<SpeechToText>,
    tts: Option<TextToSpeech>,
    event_tx: Option<mpsc::Sender<VoiceEvent>>,
    event_rx: Option<mpsc::Receiver<VoiceEvent>>,
    pub telegram_channel: Option<TelegramVoiceChannel>,
}

impl VoiceManager {
    pub fn new(config: VoiceConfig) -> Self {
        Self {
            config,
            state: VoiceState::Idle,
            recorder: None,
            wake_word_detector: None,
            stt: None,
            tts: None,
            event_tx: None,
            event_rx: None,
            telegram_channel: None,
        }
    }

    /// Initialize all voice components.
    pub async fn initialize(&mut self) -> anyhow::Result<()> {
        self.set_state(VoiceState::Idle);

        self.recorder = Some(AudioRecorder::new(self.config.audio.clone()));

        let mut detector = WakeWordDetector::new(self.config.wake_word.clone());
        detector.load_models().await?;
        self.wake_word_detector = Some(detector);

        self.stt = Some(SpeechToText::new(self.config.stt.clone()));
        self.tts = Some(TextToSpeech::new(self.config.tts.clone()));

        let (tx, rx) = mpsc::channel(100);
        self.event_tx = Some(tx);
        self.event_rx = Some(rx);

        tracing::info!("Voice manager initialized");
        Ok(())
    }

    /// Start the voice pipeline.
    pub async fn start(&mut self) -> anyhow::Result<()> {
        let detector = self
            .wake_word_detector
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Voice manager not initialized"))?;

        let mut wake_word_rx = detector.start_listening().await?;
        let tx = self
            .event_tx
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Event channel not initialized"))?
            .clone();
        let _voice_dir = self.config.voice_dir.clone();

        tokio::spawn(async move {
            while let Some(word) = wake_word_rx.recv().await {
                let _ = tx.send(VoiceEvent::WakeWordDetected { word }).await;
            }
        });

        self.set_state(VoiceState::Listening);
        tracing::info!("Voice pipeline started — listening for wake words");
        Ok(())
    }

    /// Process a voice recording: transcribe and return text.
    pub async fn process_voice(
        &mut self,
        audio_path: PathBuf,
    ) -> anyhow::Result<TranscriptionResult> {
        self.set_state(VoiceState::Transcribing);

        let stt = self
            .stt
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("STT not initialized"))?;

        let result = stt.transcribe(&audio_path).await?;

        tracing::info!("Transcription: {}", result.text);

        if let Some(tx) = &self.event_tx {
            let _ = tx
                .send(VoiceEvent::Transcription {
                    text: result.text.clone(),
                })
                .await;
        }

        self.set_state(VoiceState::Idle);
        Ok(result)
    }

    /// Speak text using TTS.
    pub async fn speak(&mut self, text: &str) -> anyhow::Result<()> {
        self.set_state(VoiceState::Speaking);

        let tts = self
            .tts
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("TTS not initialized"))?;

        tts.speak(text).await?;

        if let Some(tx) = &self.event_tx {
            let _ = tx
                .send(VoiceEvent::Speaking {
                    text: text.to_string(),
                })
                .await;
        }

        self.set_state(VoiceState::Idle);
        Ok(())
    }

    /// Stop the voice pipeline.
    pub fn stop(&mut self) {
        if let Some(detector) = &self.wake_word_detector {
            detector.stop_listening();
        }
        if let Some(recorder) = &self.recorder {
            recorder.stop_recording();
        }
        self.set_state(VoiceState::Idle);
    }

    pub fn get_event_receiver(&mut self) -> Option<mpsc::Receiver<VoiceEvent>> {
        self.event_rx.take()
    }

    fn set_state(&mut self, state: VoiceState) {
        self.state = state.clone();
        if let Some(tx) = &self.event_tx {
            let _ = tx.try_send(VoiceEvent::StateChange { state });
        }
    }

    pub fn state(&self) -> &VoiceState {
        &self.state
    }

    pub fn is_available(&self) -> bool {
        matches!(self.state, VoiceState::Idle | VoiceState::Listening)
    }
}
