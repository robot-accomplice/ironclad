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
    #[serde(default, skip_serializing)]
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

    #[test]
    fn voice_config_partial_json_applies_defaults() {
        let json = r#"{"stt_model": "custom-model"}"#;
        let config: VoiceConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.stt_model, "custom-model");
        // All other fields should get their defaults
        assert_eq!(config.tts_model, "");
        assert_eq!(config.tts_voice, "");
        assert!(!config.local_stt);
        assert!(!config.local_tts);
        assert_eq!(config.sample_rate, 16_000);
        assert!(config.api_key.is_none());
        assert_eq!(config.api_base_url, "https://api.openai.com/v1");
    }

    #[test]
    fn voice_config_empty_json_uses_all_defaults() {
        let config: VoiceConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.sample_rate, 16_000);
        assert_eq!(config.api_base_url, "https://api.openai.com/v1");
    }

    #[test]
    fn voice_config_with_api_key() {
        let json = r#"{"api_key": "sk-test123"}"#;
        let config: VoiceConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.api_key.as_deref(), Some("sk-test123"));
    }

    #[test]
    fn voice_config_api_key_not_serialized() {
        // api_key has skip_serializing, so it should not appear in output
        let mut config = VoiceConfig::default();
        config.api_key = Some("secret".into());
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("secret"), "api_key should be skipped in serialization");
    }

    #[test]
    fn voice_config_with_local_flags() {
        let json = r#"{"local_stt": true, "local_tts": true}"#;
        let config: VoiceConfig = serde_json::from_str(json).unwrap();
        assert!(config.local_stt);
        assert!(config.local_tts);
    }

    #[test]
    fn voice_config_custom_sample_rate() {
        let json = r#"{"sample_rate": 44100}"#;
        let config: VoiceConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.sample_rate, 44100);
    }

    #[test]
    fn voice_config_debug_impl() {
        let config = VoiceConfig::default();
        let debug = format!("{:?}", config);
        assert!(debug.contains("VoiceConfig"));
        assert!(debug.contains("whisper-large-v3"));
    }

    #[test]
    fn voice_config_clone() {
        let config = VoiceConfig::default();
        let cloned = config.clone();
        assert_eq!(config.stt_model, cloned.stt_model);
        assert_eq!(config.sample_rate, cloned.sample_rate);
    }

    #[test]
    fn transcription_struct_fields() {
        let t = Transcription {
            text: "hello world".into(),
            language: "en".into(),
            confidence: 0.95,
            duration_ms: 1500,
        };
        assert_eq!(t.text, "hello world");
        assert_eq!(t.language, "en");
        assert!((t.confidence - 0.95).abs() < f64::EPSILON);
        assert_eq!(t.duration_ms, 1500);
    }

    #[test]
    fn transcription_serde_roundtrip() {
        let t = Transcription {
            text: "test".into(),
            language: "fr".into(),
            confidence: 0.88,
            duration_ms: 3000,
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: Transcription = serde_json::from_str(&json).unwrap();
        assert_eq!(back.text, "test");
        assert_eq!(back.language, "fr");
        assert_eq!(back.duration_ms, 3000);
    }

    #[test]
    fn transcription_clone() {
        let t = Transcription {
            text: "hi".into(),
            language: "en".into(),
            confidence: 1.0,
            duration_ms: 100,
        };
        let cloned = t.clone();
        assert_eq!(t.text, cloned.text);
        assert_eq!(t.duration_ms, cloned.duration_ms);
    }

    #[test]
    fn synthesis_result_fields() {
        let r = SynthesisResult {
            audio_data: vec![1, 2, 3, 4],
            format: AudioFormat::Mp3,
            sample_rate: 22050,
            duration_ms: 500,
        };
        assert_eq!(r.audio_data, vec![1, 2, 3, 4]);
        assert_eq!(r.format, AudioFormat::Mp3);
        assert_eq!(r.sample_rate, 22050);
        assert_eq!(r.duration_ms, 500);
    }

    #[test]
    fn synthesis_result_clone() {
        let r = SynthesisResult {
            audio_data: vec![0u8; 100],
            format: AudioFormat::Wav,
            sample_rate: 16000,
            duration_ms: 1000,
        };
        let cloned = r.clone();
        assert_eq!(r.audio_data.len(), cloned.audio_data.len());
        assert_eq!(r.format, cloned.format);
    }

    #[test]
    fn audio_format_equality() {
        assert_eq!(AudioFormat::Wav, AudioFormat::Wav);
        assert_ne!(AudioFormat::Wav, AudioFormat::Mp3);
        assert_ne!(AudioFormat::Opus, AudioFormat::Ogg);
    }

    #[test]
    fn audio_format_copy() {
        let f = AudioFormat::Pcm;
        let f2 = f;
        assert_eq!(f, f2);
    }

    #[test]
    fn whisper_response_deserialization() {
        let json = r#"{"text": "hello world"}"#;
        let resp: WhisperResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.text, "hello world");
        assert!(resp.language.is_none());
        assert!(resp.duration.is_none());
    }

    #[test]
    fn whisper_response_full_fields() {
        let json = r#"{"text": "hi", "language": "en", "duration": 2.5}"#;
        let resp: WhisperResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.text, "hi");
        assert_eq!(resp.language.as_deref(), Some("en"));
        assert!((resp.duration.unwrap() - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn pipeline_config_accessor() {
        let config = VoiceConfig {
            stt_model: "custom-stt".into(),
            ..VoiceConfig::default()
        };
        let pipeline = VoicePipeline::new(config);
        assert_eq!(pipeline.config().stt_model, "custom-stt");
        assert_eq!(pipeline.config().tts_model, "tts-1");
    }

    #[test]
    fn pipeline_new_initializes_counters_to_zero() {
        let pipeline = VoicePipeline::new(VoiceConfig::default());
        assert_eq!(pipeline.transcription_count(), 0);
        assert_eq!(pipeline.synthesis_count(), 0);
    }

    #[test]
    fn pipeline_api_key_missing_error() {
        let pipeline = VoicePipeline::new(VoiceConfig::default());
        let result = pipeline.api_key();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("API key"));
    }

    #[test]
    fn pipeline_api_key_present() {
        let config = VoiceConfig {
            api_key: Some("sk-test".into()),
            ..VoiceConfig::default()
        };
        let pipeline = VoicePipeline::new(config);
        assert_eq!(pipeline.api_key().unwrap(), "sk-test");
    }

    #[test]
    fn default_sample_rate_value() {
        assert_eq!(default_sample_rate(), 16_000);
    }

    #[test]
    fn default_api_base_url_value() {
        assert_eq!(default_api_base_url(), "https://api.openai.com/v1");
    }

    fn fast_fail_pipeline() -> VoicePipeline {
        let config = VoiceConfig {
            api_key: Some("sk-test".into()),
            api_base_url: "http://127.0.0.1:1".into(),
            ..VoiceConfig::default()
        };
        let mut pipeline = VoicePipeline::new(config);
        pipeline.http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(50))
            .build()
            .unwrap();
        pipeline
    }

    #[tokio::test]
    async fn transcribe_network_error() {
        let mut pipeline = fast_fail_pipeline();
        let audio = vec![0u8; 1000];
        let result = pipeline.transcribe(&audio, AudioFormat::Wav).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("whisper request failed"),
            "unexpected error: {err}"
        );
        // Verify counter was still incremented
        assert_eq!(pipeline.transcription_count(), 1);
    }

    #[tokio::test]
    async fn transcribe_network_error_with_each_format() {
        for format in [AudioFormat::Opus, AudioFormat::Mp3, AudioFormat::Ogg, AudioFormat::Pcm] {
            let mut pipeline = fast_fail_pipeline();
            let result = pipeline.transcribe(&[1, 2, 3], format).await;
            assert!(result.is_err());
        }
    }

    #[tokio::test]
    async fn synthesize_network_error() {
        let mut pipeline = fast_fail_pipeline();
        let result = pipeline.synthesize("Hello world").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("TTS request failed"),
            "unexpected error: {err}"
        );
        // Verify counter was still incremented
        assert_eq!(pipeline.synthesis_count(), 1);
    }

    #[tokio::test]
    async fn synthesize_empty_text_network_error() {
        let mut pipeline = fast_fail_pipeline();
        let result = pipeline.synthesize("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn synthesize_long_text_network_error() {
        let mut pipeline = fast_fail_pipeline();
        let long_text = "word ".repeat(100);
        let result = pipeline.synthesize(&long_text).await;
        assert!(result.is_err());
    }
}
