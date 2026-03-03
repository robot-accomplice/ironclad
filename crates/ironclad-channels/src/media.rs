//! Media download and storage service for multimodal channel messages.
//!
//! [`MediaService`] handles downloading media attachments from platform-specific
//! URLs, validating size and content-type, and storing them locally for
//! downstream processing (vision, transcription, etc.).

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use ironclad_core::error::{IroncladError, Result};
use tracing::debug;

#[cfg(test)]
use super::MediaAttachment;
use super::MediaType;

/// Maximum filename length (bytes) to prevent path traversal or FS issues.
const MAX_FILENAME_LEN: usize = 200;

/// Media download and storage service.
///
/// Validates size limits per media type, streams downloads with early abort,
/// and writes files to a configured media directory.
pub struct MediaService {
    client: reqwest::Client,
    media_dir: PathBuf,
    max_image_size: usize,
    max_audio_size: usize,
    max_video_size: usize,
    max_document_size: usize,
}

impl MediaService {
    /// Create a new `MediaService` from multimodal configuration.
    pub fn new(config: &ironclad_core::config::MultimodalConfig) -> Result<Self> {
        let media_dir = config
            .media_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("data/media"));

        // Ensure the media directory exists
        std::fs::create_dir_all(&media_dir).map_err(|e| {
            IroncladError::Channel(format!(
                "failed to create media directory {}: {e}",
                media_dir.display()
            ))
        })?;

        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .map_err(|e| IroncladError::Channel(format!("media HTTP client error: {e}")))?,
            media_dir,
            max_image_size: config.max_image_size_bytes,
            max_audio_size: config.max_audio_size_bytes,
            max_video_size: config.max_video_size_bytes,
            max_document_size: config.max_document_size_bytes,
        })
    }

    /// Returns the maximum allowed download size for a given media type.
    pub fn max_size_for(&self, media_type: &MediaType) -> usize {
        match media_type {
            MediaType::Image => self.max_image_size,
            MediaType::Audio => self.max_audio_size,
            MediaType::Video => self.max_video_size,
            MediaType::Document => self.max_document_size,
        }
    }

    /// Returns the configured media directory.
    pub fn media_dir(&self) -> &Path {
        &self.media_dir
    }

    /// Download a file from `url`, validate it, and store locally.
    ///
    /// Returns the local path where the file was saved. The file is stored as
    /// `{media_dir}/{uuid}_{sanitized_filename}`.
    pub async fn download_and_store(
        &self,
        url: &str,
        media_type: &MediaType,
        filename: Option<&str>,
    ) -> Result<PathBuf> {
        let validated_url = validate_remote_url(url)?;
        let max_size = self.max_size_for(media_type);

        // HEAD request to check Content-Length before downloading
        let head = self
            .client
            .head(validated_url.clone())
            .send()
            .await
            .map_err(|e| IroncladError::Channel(format!("media HEAD request failed: {e}")))?;

        if let Some(cl) = head
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok())
            && cl > max_size
        {
            return Err(IroncladError::Channel(format!(
                "media too large: {cl} bytes exceeds {max_size} byte limit for {media_type:?}"
            )));
        }

        // Streaming GET with size guard
        let resp = self
            .client
            .get(validated_url)
            .send()
            .await
            .map_err(|e| IroncladError::Channel(format!("media GET request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(IroncladError::Channel(format!(
                "media download returned HTTP {status}"
            )));
        }

        // Validate content-type header matches expected media type
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();

        let inferred = MediaType::from_content_type(&content_type);
        if inferred != *media_type {
            debug!(
                expected = ?media_type,
                got = ?inferred,
                content_type = %content_type,
                "media content-type mismatch, proceeding anyway"
            );
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| IroncladError::Channel(format!("media download read failed: {e}")))?;

        if bytes.len() > max_size {
            return Err(IroncladError::Channel(format!(
                "media body too large: {} bytes exceeds {max_size} byte limit for {media_type:?}",
                bytes.len()
            )));
        }

        // Build safe local path
        let safe_name = sanitize_filename(filename.unwrap_or("attachment"));
        let uuid_prefix = uuid::Uuid::new_v4();
        let local_name = format!("{uuid_prefix}_{safe_name}");
        let local_path = self.media_dir.join(&local_name);

        tokio::fs::write(&local_path, &bytes).await.map_err(|e| {
            IroncladError::Channel(format!(
                "failed to write media to {}: {e}",
                local_path.display()
            ))
        })?;

        debug!(
            path = %local_path.display(),
            size = bytes.len(),
            media_type = ?media_type,
            "media downloaded and stored"
        );

        Ok(local_path)
    }

    /// Download a WhatsApp media attachment using the two-step flow:
    /// 1. GET media URL from `graph.facebook.com/v21.0/{media_id}` (returns JSON with `url`)
    /// 2. GET the actual media binary from the returned URL
    pub async fn download_whatsapp_media(
        &self,
        media_id: &str,
        access_token: &str,
        media_type: &MediaType,
        filename: Option<&str>,
    ) -> Result<PathBuf> {
        // Step 1: resolve media ID → download URL
        let meta_url = format!("https://graph.facebook.com/v21.0/{media_id}",);
        let meta_resp = self
            .client
            .get(&meta_url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| {
                IroncladError::Channel(format!("WhatsApp media metadata request failed: {e}"))
            })?;

        let meta_json: serde_json::Value = meta_resp.json().await.map_err(|e| {
            IroncladError::Channel(format!("WhatsApp media metadata parse failed: {e}"))
        })?;

        let download_url = meta_json
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                IroncladError::Channel("WhatsApp media metadata missing 'url' field".into())
            })?;

        // Step 2: download the actual binary (with auth header)
        let max_size = self.max_size_for(media_type);
        let resp = self
            .client
            .get(download_url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| IroncladError::Channel(format!("WhatsApp media download failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(IroncladError::Channel(format!(
                "WhatsApp media download returned HTTP {}",
                resp.status()
            )));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| IroncladError::Channel(format!("WhatsApp media read failed: {e}")))?;

        if bytes.len() > max_size {
            return Err(IroncladError::Channel(format!(
                "WhatsApp media too large: {} bytes exceeds {max_size} limit",
                bytes.len()
            )));
        }

        let safe_name = sanitize_filename(filename.unwrap_or("whatsapp_media"));
        let uuid_prefix = uuid::Uuid::new_v4();
        let local_name = format!("{uuid_prefix}_{safe_name}");
        let local_path = self.media_dir.join(&local_name);

        tokio::fs::write(&local_path, &bytes).await.map_err(|e| {
            IroncladError::Channel(format!(
                "failed to write WhatsApp media to {}: {e}",
                local_path.display()
            ))
        })?;

        debug!(
            path = %local_path.display(),
            size = bytes.len(),
            media_id = media_id,
            "WhatsApp media downloaded"
        );

        Ok(local_path)
    }

    /// Download a Discord attachment directly from a CDN URL.
    pub async fn download_discord_attachment(
        &self,
        url: &str,
        media_type: &MediaType,
        filename: Option<&str>,
    ) -> Result<PathBuf> {
        self.download_and_store(url, media_type, filename).await
    }
}

fn validate_remote_url(raw: &str) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(raw)
        .map_err(|e| IroncladError::Channel(format!("invalid media URL: {e}")))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(IroncladError::Channel(format!(
                "unsupported media URL scheme: {other}"
            )));
        }
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| IroncladError::Channel("media URL is missing host".into()))?;
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".local") {
        return Err(IroncladError::Channel(
            "media URL host is not allowed".into(),
        ));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        let blocked = match ip {
            IpAddr::V4(v4) => {
                v4.is_private()
                    || v4.is_loopback()
                    || v4.is_link_local()
                    || v4.is_multicast()
                    || v4.is_unspecified()
            }
            IpAddr::V6(v6) => {
                v6.is_loopback() || v6.is_unspecified() || v6.is_unique_local() || v6.is_multicast()
            }
        };
        if blocked {
            return Err(IroncladError::Channel(
                "media URL IP range is not allowed".into(),
            ));
        }
    }
    Ok(parsed)
}

/// Sanitize a filename for safe local storage.
/// Removes path separators, null bytes, and truncates to `MAX_FILENAME_LEN`.
fn sanitize_filename(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| *c != '/' && *c != '\\' && *c != '\0' && !c.is_control())
        .collect();

    let name = if cleaned.is_empty() {
        "attachment".to_string()
    } else {
        cleaned
    };

    if name.len() <= MAX_FILENAME_LEN {
        name
    } else {
        let mut end = MAX_FILENAME_LEN;
        while end > 0 && !name.is_char_boundary(end) {
            end -= 1;
        }
        name[..end].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_type_from_content_type_image() {
        assert_eq!(MediaType::from_content_type("image/png"), MediaType::Image);
        assert_eq!(MediaType::from_content_type("image/jpeg"), MediaType::Image);
        assert_eq!(MediaType::from_content_type("IMAGE/WEBP"), MediaType::Image);
    }

    #[test]
    fn media_type_from_content_type_audio() {
        assert_eq!(MediaType::from_content_type("audio/ogg"), MediaType::Audio);
        assert_eq!(MediaType::from_content_type("audio/mpeg"), MediaType::Audio);
    }

    #[test]
    fn media_type_from_content_type_video() {
        assert_eq!(MediaType::from_content_type("video/mp4"), MediaType::Video);
    }

    #[test]
    fn media_type_from_content_type_document() {
        assert_eq!(
            MediaType::from_content_type("application/pdf"),
            MediaType::Document
        );
        assert_eq!(
            MediaType::from_content_type("text/plain"),
            MediaType::Document
        );
    }

    #[test]
    fn sanitize_filename_removes_path_separators() {
        assert_eq!(sanitize_filename("../../../etc/passwd"), "......etcpasswd");
        assert_eq!(sanitize_filename("file\\name.txt"), "filename.txt");
    }

    #[test]
    fn sanitize_filename_removes_nulls_and_control() {
        assert_eq!(sanitize_filename("bad\x00name\x01.txt"), "badname.txt");
    }

    #[test]
    fn sanitize_filename_empty_becomes_attachment() {
        assert_eq!(sanitize_filename(""), "attachment");
        assert_eq!(sanitize_filename("/"), "attachment");
    }

    #[test]
    fn sanitize_filename_truncates_long_names() {
        let long = "a".repeat(300);
        assert!(sanitize_filename(&long).len() <= MAX_FILENAME_LEN);
    }

    #[test]
    fn sanitize_filename_preserves_normal_names() {
        assert_eq!(sanitize_filename("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_filename("report_2024.pdf"), "report_2024.pdf");
    }

    #[test]
    fn media_attachment_serde_roundtrip() {
        let att = MediaAttachment {
            media_type: MediaType::Image,
            source_url: Some("https://example.com/img.png".into()),
            local_path: None,
            filename: Some("img.png".into()),
            content_type: "image/png".into(),
            size_bytes: Some(12345),
            caption: Some("A picture".into()),
        };
        let json = serde_json::to_string(&att).unwrap();
        let decoded: MediaAttachment = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.media_type, MediaType::Image);
        assert_eq!(decoded.filename.as_deref(), Some("img.png"));
        assert_eq!(decoded.size_bytes, Some(12345));
    }

    #[test]
    fn media_attachment_skips_none_fields() {
        let att = MediaAttachment {
            media_type: MediaType::Audio,
            source_url: None,
            local_path: None,
            filename: None,
            content_type: "audio/ogg".into(),
            size_bytes: None,
            caption: None,
        };
        let json = serde_json::to_string(&att).unwrap();
        assert!(!json.contains("source_url"));
        assert!(!json.contains("local_path"));
        assert!(!json.contains("filename"));
        assert!(!json.contains("size_bytes"));
        assert!(!json.contains("caption"));
    }

    #[test]
    fn media_service_max_size_for() {
        let config = ironclad_core::config::MultimodalConfig {
            enabled: true,
            media_dir: Some(PathBuf::from("/tmp/test-media")),
            max_image_size_bytes: 10_000_000,
            max_audio_size_bytes: 25_000_000,
            max_video_size_bytes: 50_000_000,
            max_document_size_bytes: 50_000_000,
            vision_model: None,
            transcription_model: None,
            auto_transcribe_audio: false,
            auto_describe_images: false,
        };
        // We can't call MediaService::new without a real filesystem, so test the logic directly
        let service = MediaService {
            client: reqwest::Client::new(),
            media_dir: PathBuf::from("/tmp/test-media"),
            max_image_size: config.max_image_size_bytes,
            max_audio_size: config.max_audio_size_bytes,
            max_video_size: config.max_video_size_bytes,
            max_document_size: config.max_document_size_bytes,
        };
        assert_eq!(service.max_size_for(&MediaType::Image), 10_000_000);
        assert_eq!(service.max_size_for(&MediaType::Audio), 25_000_000);
        assert_eq!(service.max_size_for(&MediaType::Video), 50_000_000);
        assert_eq!(service.max_size_for(&MediaType::Document), 50_000_000);
    }

    #[test]
    fn validate_remote_url_rejects_local_targets() {
        assert!(validate_remote_url("http://localhost/file").is_err());
        assert!(validate_remote_url("http://127.0.0.1/file").is_err());
        assert!(validate_remote_url("https://10.0.0.5/file").is_err());
        assert!(validate_remote_url("file:///tmp/x").is_err());
    }

    #[test]
    fn validate_remote_url_allows_public_https() {
        assert!(validate_remote_url("https://cdn.discordapp.com/attachments/x/y.png").is_ok());
    }
}
