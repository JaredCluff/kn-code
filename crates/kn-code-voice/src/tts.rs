use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Text-to-speech engine.
/// Supports Piper TTS (local, high-quality neural) and eSpeak (lightweight fallback).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsConfig {
    pub engine: TtsEngine,
    pub voice: Option<String>,
    pub speed: f32,
    pub output_dir: Option<PathBuf>,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            engine: TtsEngine::default(),
            voice: None,
            speed: 1.0,
            output_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TtsEngine {
    #[default]
    Piper,
    Espeak,
    System,
}

pub struct TextToSpeech {
    config: TtsConfig,
}

impl TextToSpeech {
    pub fn new(config: TtsConfig) -> Self {
        Self { config }
    }

    /// Synthesize text to a WAV file.
    pub async fn synthesize_to_file(
        &self,
        text: &str,
        output_path: PathBuf,
    ) -> anyhow::Result<PathBuf> {
        match &self.config.engine {
            TtsEngine::Piper => self.synthesize_piper(text, &output_path).await,
            TtsEngine::Espeak => self.synthesize_espeak(text, &output_path).await,
            TtsEngine::System => self.synthesize_system(text, &output_path).await,
        }
    }

    /// Synthesize and play audio directly (no file).
    pub async fn speak(&self, text: &str) -> anyhow::Result<()> {
        let temp_path = self
            .config
            .output_dir
            .clone()
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("kn-tts-{}.wav", std::process::id()));

        self.synthesize_to_file(text, temp_path.clone()).await?;

        // Play the audio
        self.play_audio(&temp_path).await?;

        let _ = tokio::fs::remove_file(&temp_path).await;
        Ok(())
    }

    async fn synthesize_piper(&self, _text: &str, _output: &Path) -> anyhow::Result<PathBuf> {
        anyhow::bail!("Piper TTS engine is not yet implemented — use espeak or system TTS instead")
    }

    async fn synthesize_espeak(&self, text: &str, output: &Path) -> anyhow::Result<PathBuf> {
        let speed = (self.config.speed * 100.0) as u32;
        let voice = self.config.voice.as_deref().unwrap_or("en-us");

        let result = tokio::process::Command::new("espeak")
            .args([
                "-w",
                output
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("Invalid output path"))?,
                "-s",
                &speed.to_string(),
                "-v",
                voice,
                text,
            ])
            .output()
            .await?;

        if !result.status.success() {
            anyhow::bail!("eSpeak failed: {}", String::from_utf8_lossy(&result.stderr));
        }

        Ok(output.to_path_buf())
    }

    async fn synthesize_system(&self, text: &str, output: &Path) -> anyhow::Result<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            let result = tokio::process::Command::new("say")
                .args(["-o", output.to_str().unwrap(), text])
                .output()
                .await?;
            if !result.status.success() {
                anyhow::bail!(
                    "say command failed: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
            Ok(output.to_path_buf())
        }
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("System TTS not supported on this platform");
        }
    }

    async fn play_audio(&self, path: &Path) -> anyhow::Result<()> {
        #[cfg(target_os = "macos")]
        {
            let result = tokio::process::Command::new("afplay")
                .arg(path)
                .output()
                .await?;
            if !result.status.success() {
                anyhow::bail!("afplay failed: {}", String::from_utf8_lossy(&result.stderr));
            }
        }
        #[cfg(target_os = "linux")]
        {
            let result = tokio::process::Command::new("aplay")
                .arg(path)
                .output()
                .await?;
            if !result.status.success() {
                anyhow::bail!("aplay failed: {}", String::from_utf8_lossy(&result.stderr));
            }
        }
        Ok(())
    }
}
