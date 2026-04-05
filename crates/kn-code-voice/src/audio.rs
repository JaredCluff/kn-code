use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::WavWriter;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub device_name: Option<String>,
    pub silence_threshold: f32,
    pub silence_duration_ms: u64,
    pub max_recording_secs: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            channels: 1,
            bits_per_sample: 16,
            device_name: None,
            silence_threshold: 0.01,
            silence_duration_ms: 1500,
            max_recording_secs: 60,
        }
    }
}

pub struct AudioRecorder {
    config: AudioConfig,
    recording: Arc<AtomicBool>,
}

impl AudioRecorder {
    pub fn new(config: AudioConfig) -> Self {
        Self {
            config,
            recording: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn list_devices() -> Vec<String> {
        let host = cpal::default_host();
        match host.input_devices() {
            Ok(devices) => devices
                .filter_map(|d: cpal::Device| d.name().ok())
                .collect(),
            Err(e) => {
                tracing::warn!("Failed to get input devices: {}", e);
                Vec::new()
            }
        }
    }

    pub fn get_default_device() -> Option<cpal::Device> {
        let host = cpal::default_host();
        host.default_input_device()
    }

    pub fn get_device(&self) -> Option<cpal::Device> {
        if let Some(name) = &self.config.device_name {
            let host = cpal::default_host();
            match host.input_devices() {
                Ok(mut devices) => {
                    devices.find(|d: &cpal::Device| d.name().ok().as_ref() == Some(name))
                }
                Err(e) => {
                    tracing::warn!("Failed to get input devices: {}", e);
                    Self::get_default_device()
                }
            }
        } else {
            Self::get_default_device()
        }
    }

    /// Record audio until silence is detected or max duration is reached.
    /// Returns the path to the saved WAV file.
    ///
    /// Runs in spawn_blocking since cpal::Stream is not Send on macOS.
    pub async fn record_until_silence(&self, output_path: PathBuf) -> anyhow::Result<PathBuf> {
        let device = self
            .get_device()
            .ok_or_else(|| anyhow::anyhow!("No input device available"))?;

        let config = device
            .default_input_config()
            .map_err(|e| anyhow::anyhow!("Failed to get input config: {}", e))?;

        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let silence_threshold = self.config.silence_threshold;
        let silence_duration_ms = self.config.silence_duration_ms;
        let max_samples = (sample_rate as u64 * self.config.max_recording_secs as u64) as usize;
        let max_recording_secs = self.config.max_recording_secs;
        let recording = self.recording.clone();

        let result_path = output_path.clone();
        tokio::task::spawn_blocking(move || {
            let spec = hound::WavSpec {
                channels,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };

            let mut writer = WavWriter::create(&result_path, spec)
                .map_err(|e| anyhow::anyhow!("Failed to create WAV file: {}", e))?;

            recording.store(true, Ordering::SeqCst);

            let (tx, rx) = std::sync::mpsc::channel::<i16>();
            let recording_clone = recording.clone();

            let err_fn = move |err| {
                tracing::error!("Audio capture error: {}", err);
            };

            let stream = device
                .build_input_stream(
                    &config.into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if recording_clone.load(Ordering::SeqCst) {
                            for &sample in data {
                                let _ = tx.send(sample);
                            }
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| anyhow::anyhow!("Failed to build input stream: {}", e))?;

            stream
                .play()
                .map_err(|e| anyhow::anyhow!("Failed to start stream: {}", e))?;

            let mut samples = Vec::new();
            let mut silence_start: Option<std::time::Instant> = None;
            let start_time = std::time::Instant::now();

            loop {
                if start_time.elapsed().as_secs() >= max_recording_secs as u64 {
                    break;
                }

                match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(sample) => {
                        let amplitude = (sample as f32 / i16::MAX as f32).abs();

                        if amplitude > silence_threshold {
                            silence_start = None;
                        } else if let Some(start) = silence_start {
                            if start.elapsed().as_millis() >= silence_duration_ms as u128 {
                                break;
                            }
                        } else {
                            silence_start = Some(std::time::Instant::now());
                        }

                        samples.push(sample);

                        if samples.len() >= max_samples {
                            break;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                }
            }

            recording.store(false, Ordering::SeqCst);
            drop(stream);

            for sample in &samples {
                writer
                    .write_sample(*sample)
                    .map_err(|e| anyhow::anyhow!("Failed to write sample: {}", e))?;
            }
            writer
                .finalize()
                .map_err(|e| anyhow::anyhow!("Failed to finalize WAV file: {}", e))?;

            tracing::info!("Recorded {} samples to {:?}", samples.len(), result_path);
            Ok::<_, anyhow::Error>(result_path)
        })
        .await?
    }

    pub fn stop_recording(&self) {
        self.recording.store(false, Ordering::SeqCst);
    }

    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::SeqCst)
    }
}
