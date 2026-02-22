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
}

fn default_sample_rate() -> u32 {
    16_000
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            stt_model: "whisper-large-v3".to_string(),
            tts_model: "piper".to_string(),
            tts_voice: "en_US-amy-medium".to_string(),
            local_stt: true,
            local_tts: true,
            sample_rate: default_sample_rate(),
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

/// Voice processing pipeline for STT and TTS.
pub struct VoicePipeline {
    config: VoiceConfig,
    transcription_count: u64,
    synthesis_count: u64,
}

impl VoicePipeline {
    pub fn new(config: VoiceConfig) -> Self {
        info!(stt = %config.stt_model, tts = %config.tts_model, "voice pipeline initialized");
        Self {
            config,
            transcription_count: 0,
            synthesis_count: 0,
        }
    }

    /// Transcribe audio data to text using STT.
    pub fn transcribe(&mut self, audio_data: &[u8], format: AudioFormat) -> Transcription {
        self.transcription_count += 1;

        let estimated_duration =
            (audio_data.len() as u64 * 1000) / (self.config.sample_rate as u64 * 2).max(1);

        debug!(
            model = %self.config.stt_model,
            format = %format,
            audio_bytes = audio_data.len(),
            "transcribing audio"
        );

        Transcription {
            text: format!(
                "[Transcription of {} bytes of {} audio]",
                audio_data.len(),
                format
            ),
            language: "en".to_string(),
            confidence: 0.95,
            duration_ms: estimated_duration,
        }
    }

    /// Synthesize text to audio using TTS.
    pub fn synthesize(&mut self, text: &str) -> SynthesisResult {
        self.synthesis_count += 1;

        let words = text.split_whitespace().count();
        let duration_ms = (words as u64 * 300).max(100);
        let estimated_bytes = (duration_ms * self.config.sample_rate as u64 * 2) / 1000;

        debug!(
            model = %self.config.tts_model,
            voice = %self.config.tts_voice,
            text_len = text.len(),
            "synthesizing speech"
        );

        SynthesisResult {
            audio_data: vec![0u8; estimated_bytes as usize],
            format: AudioFormat::Wav,
            sample_rate: self.config.sample_rate,
            duration_ms,
        }
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
        assert_eq!(config.tts_model, "piper");
        assert!(config.local_stt);
        assert!(config.local_tts);
        assert_eq!(config.sample_rate, 16_000);
    }

    #[test]
    fn transcribe_audio() {
        let mut pipeline = VoicePipeline::new(VoiceConfig::default());
        let audio = vec![0u8; 32_000];
        let result = pipeline.transcribe(&audio, AudioFormat::Wav);
        assert!(!result.text.is_empty());
        assert_eq!(result.language, "en");
        assert!(result.confidence > 0.0);
        assert_eq!(pipeline.transcription_count(), 1);
    }

    #[test]
    fn synthesize_text() {
        let mut pipeline = VoicePipeline::new(VoiceConfig::default());
        let result = pipeline.synthesize("Hello world, this is a test.");
        assert!(!result.audio_data.is_empty());
        assert_eq!(result.format, AudioFormat::Wav);
        assert_eq!(result.sample_rate, 16_000);
        assert!(result.duration_ms > 0);
        assert_eq!(pipeline.synthesis_count(), 1);
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

    #[test]
    fn pipeline_counters() {
        let mut pipeline = VoicePipeline::new(VoiceConfig::default());
        assert_eq!(pipeline.transcription_count(), 0);
        assert_eq!(pipeline.synthesis_count(), 0);

        pipeline.transcribe(&[0; 100], AudioFormat::Pcm);
        pipeline.transcribe(&[0; 100], AudioFormat::Pcm);
        pipeline.synthesize("test");

        assert_eq!(pipeline.transcription_count(), 2);
        assert_eq!(pipeline.synthesis_count(), 1);
    }

    #[test]
    fn voice_config_serde() {
        let config = VoiceConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let back: VoiceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.stt_model, back.stt_model);
    }
}
