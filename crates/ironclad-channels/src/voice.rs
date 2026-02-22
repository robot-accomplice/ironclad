use ironclad_core::{IroncladError, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Voice channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    #[serde(default)]
    pub stt_model: String,
    #[serde(default)]
    pub tts_model: String,
    #[serde(default)]
    pub tts_voice: String,
    #[serde(default)]
    pub local_stt: bool,
    #[serde(default)]
    pub local_tts: bool,
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_api_base_url")]
    pub api_base_url: String,
}

fn default_sample_rate() -> u32 {
    16_000
}

fn default_api_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            stt_model: "whisper-large-v3".to_string(),
            tts_model: "tts-1".to_string(),
            tts_voice: "alloy".to_string(),
            local_stt: false,
            local_tts: false,
            sample_rate: default_sample_rate(),
            api_key: None,
            api_base_url: default_api_base_url(),
        }
    }
}

/// Result of speech-to-text transcription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcription {
    pub text: String,
    pub language: String,
    pub confidence: f64,
    pub duration_ms: u64,
}

/// Result of text-to-speech synthesis.
#[derive(Debug, Clone)]
pub struct SynthesisResult {
    pub audio_data: Vec<u8>,
    pub format: AudioFormat,
    pub sample_rate: u32,
    pub duration_ms: u64,
}

/// Supported audio formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioFormat {
    Wav,
    Opus,
    Mp3,
    Ogg,
    Pcm,
}

impl std::fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioFormat::Wav => write!(f, "wav"),
            AudioFormat::Opus => write!(f, "opus"),
            AudioFormat::Mp3 => write!(f, "mp3"),
            AudioFormat::Ogg => write!(f, "ogg"),
            AudioFormat::Pcm => write!(f, "pcm"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct WhisperResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
}

/// Voice processing pipeline for STT and TTS.
pub struct VoicePipeline {
    config: VoiceConfig,
    http: reqwest::Client,
    transcription_count: u64,
    synthesis_count: u64,
}

impl VoicePipeline {
    pub fn new(config: VoiceConfig) -> Self {
        info!(stt = %config.stt_model, tts = %config.tts_model, "voice pipeline initialized");
        Self {
            config,
            http: reqwest::Client::new(),
            transcription_count: 0,
            synthesis_count: 0,
        }
    }

    fn api_key(&self) -> Result<&str> {
        self.config
            .api_key
            .as_deref()
            .ok_or_else(|| IroncladError::Config("voice API key not configured".to_string()))
    }

    /// Transcribe audio data to text using OpenAI Whisper API.
    pub async fn transcribe(
        &mut self,
        audio_data: &[u8],
        format: AudioFormat,
    ) -> Result<Transcription> {
        self.transcription_count += 1;
        let api_key = self.api_key()?.to_string();

        debug!(
            model = %self.config.stt_model,
            format = %format,
            audio_bytes = audio_data.len(),
            "transcribing audio"
        );

        let filename = format!("audio.{format}");
        let file_part = reqwest::multipart::Part::bytes(audio_data.to_vec())
            .file_name(filename)
            .mime_str("application/octet-stream")
            .map_err(|e| IroncladError::Channel(format!("multipart error: {e}")))?;

        let form = reqwest::multipart::Form::new()
            .text("model", self.config.stt_model.clone())
            .text("response_format", "verbose_json")
            .part("file", file_part);

        let url = format!("{}/audio/transcriptions", self.config.api_base_url);

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("whisper request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(IroncladError::Channel(format!(
                "whisper API returned {status}: {body}"
            )));
        }

        let whisper: WhisperResponse = resp
            .json()
            .await
            .map_err(|e| IroncladError::Channel(format!("whisper response parse error: {e}")))?;

        Ok(Transcription {
            text: whisper.text,
            language: whisper.language.unwrap_or_else(|| "en".to_string()),
            confidence: 1.0,
            duration_ms: whisper.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0),
        })
    }

    /// Synthesize text to audio using OpenAI TTS API.
    pub async fn synthesize(&mut self, text: &str) -> Result<SynthesisResult> {
        self.synthesis_count += 1;
        let api_key = self.api_key()?.to_string();

        debug!(
            model = %self.config.tts_model,
            voice = %self.config.tts_voice,
            text_len = text.len(),
            "synthesizing speech"
        );

        let url = format!("{}/audio/speech", self.config.api_base_url);

        let body = serde_json::json!({
            "model": self.config.tts_model,
            "voice": self.config.tts_voice,
            "input": text,
        });

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("TTS request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(IroncladError::Channel(format!(
                "TTS API returned {status}: {body}"
            )));
        }

        let audio_data = resp
            .bytes()
            .await
            .map_err(|e| IroncladError::Network(format!("TTS response read error: {e}")))?
            .to_vec();

        let words = text.split_whitespace().count();
        let estimated_duration_ms = (words as u64 * 300).max(100);

        Ok(SynthesisResult {
            audio_data,
            format: AudioFormat::Mp3,
            sample_rate: self.config.sample_rate,
            duration_ms: estimated_duration_ms,
        })
    }

    pub fn config(&self) -> &VoiceConfig {
        &self.config
    }

    pub fn transcription_count(&self) -> u64 {
        self.transcription_count
    }

    pub fn synthesis_count(&self) -> u64 {
        self.synthesis_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_config_defaults() {
        let config = VoiceConfig::default();
        assert_eq!(config.stt_model, "whisper-large-v3");
        assert_eq!(config.tts_model, "tts-1");
        assert!(!config.local_stt);
        assert!(!config.local_tts);
        assert_eq!(config.sample_rate, 16_000);
        assert!(config.api_key.is_none());
        assert_eq!(config.api_base_url, "https://api.openai.com/v1");
    }

    #[tokio::test]
    async fn transcribe_fails_without_api_key() {
        let mut pipeline = VoicePipeline::new(VoiceConfig::default());
        let audio = vec![0u8; 32_000];
        let result = pipeline.transcribe(&audio, AudioFormat::Wav).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("API key"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn synthesize_fails_without_api_key() {
        let mut pipeline = VoicePipeline::new(VoiceConfig::default());
        let result = pipeline.synthesize("Hello world").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("API key"), "unexpected error: {err}");
    }

    #[test]
    fn audio_format_display() {
        assert_eq!(format!("{}", AudioFormat::Wav), "wav");
        assert_eq!(format!("{}", AudioFormat::Opus), "opus");
        assert_eq!(format!("{}", AudioFormat::Mp3), "mp3");
        assert_eq!(format!("{}", AudioFormat::Ogg), "ogg");
        assert_eq!(format!("{}", AudioFormat::Pcm), "pcm");
    }

    #[test]
    fn audio_format_serde() {
        for format in [
            AudioFormat::Wav,
            AudioFormat::Opus,
            AudioFormat::Mp3,
            AudioFormat::Ogg,
            AudioFormat::Pcm,
        ] {
            let json = serde_json::to_string(&format).unwrap();
            let back: AudioFormat = serde_json::from_str(&json).unwrap();
            assert_eq!(format, back);
        }
    }

    #[tokio::test]
    async fn pipeline_counters() {
        let mut pipeline = VoicePipeline::new(VoiceConfig::default());
        assert_eq!(pipeline.transcription_count(), 0);
        assert_eq!(pipeline.synthesis_count(), 0);

        let _ = pipeline.transcribe(&[0; 100], AudioFormat::Pcm).await;
        let _ = pipeline.transcribe(&[0; 100], AudioFormat::Pcm).await;
        let _ = pipeline.synthesize("test").await;

        assert_eq!(pipeline.transcription_count(), 2);
        assert_eq!(pipeline.synthesis_count(), 1);
    }

    #[test]
    fn voice_config_serde() {
        let config = VoiceConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let back: VoiceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.stt_model, back.stt_model);
        assert_eq!(back.api_base_url, "https://api.openai.com/v1");
    }
}
