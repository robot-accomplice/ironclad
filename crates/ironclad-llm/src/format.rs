use serde::{Deserialize, Serialize};
use serde_json::Value;

use ironclad_core::{ApiFormat, IroncladError, Result};

/// Saturating cast from u64 to u32 — caps at u32::MAX instead of wrapping.
fn saturating_u32(v: u64) -> u32 {
    v.min(u32::MAX as u64) as u32
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedRequest {
    pub model: String,
    pub messages: Vec<UnifiedMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_target: Option<f64>,
}

/// Represents a content part in a multimodal message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { url: String, detail: Option<String> },
    ImageBase64 { media_type: String, data: String },
    AudioTranscription { text: String, source: String },
}

impl ContentPart {
    pub fn text(s: &str) -> Self {
        ContentPart::Text {
            text: s.to_string(),
        }
    }

    pub fn image_url(url: &str) -> Self {
        ContentPart::ImageUrl {
            url: url.to_string(),
            detail: None,
        }
    }

    pub fn image_base64(media_type: &str, data: &str) -> Self {
        ContentPart::ImageBase64 {
            media_type: media_type.to_string(),
            data: data.to_string(),
        }
    }

    pub fn audio_transcription(text: &str, source: &str) -> Self {
        ContentPart::AudioTranscription {
            text: text.to_string(),
            source: source.to_string(),
        }
    }

    pub fn to_text(&self) -> String {
        match self {
            ContentPart::Text { text } => text.clone(),
            ContentPart::ImageUrl { url, .. } => format!("[Image: {url}]"),
            ContentPart::ImageBase64 { media_type, .. } => format!("[Image: {media_type}]"),
            ContentPart::AudioTranscription { text, source } => {
                format!("[Audio from {source}]: {text}")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnifiedMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<ContentPart>>,
}

impl UnifiedMessage {
    pub fn text(role: &str, content: &str) -> Self {
        Self {
            role: role.to_string(),
            content: content.to_string(),
            parts: None,
        }
    }

    pub fn multimodal(role: &str, parts: Vec<ContentPart>) -> Self {
        let text_content = parts
            .iter()
            .map(|p| p.to_text())
            .collect::<Vec<_>>()
            .join("\n");
        Self {
            role: role.to_string(),
            content: text_content,
            parts: Some(parts),
        }
    }

    pub fn is_multimodal(&self) -> bool {
        self.parts.as_ref().is_some_and(|p| {
            p.iter()
                .any(|part| !matches!(part, ContentPart::Text { .. }))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedResponse {
    pub content: String,
    pub model: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub finish_reason: Option<String>,
}

/// A single chunk from a streaming LLM response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    pub delta: String,
    pub model: Option<String>,
    pub finish_reason: Option<String>,
    pub tokens_in: Option<u32>,
    pub tokens_out: Option<u32>,
}

/// Accumulates chunks into a final UnifiedResponse.
#[derive(Debug, Default)]
pub struct StreamAccumulator {
    content: String,
    model: Option<String>,
    tokens_in: u32,
    tokens_out: u32,
    finish_reason: Option<String>,
}

impl StreamAccumulator {
    pub fn push(&mut self, chunk: &StreamChunk) {
        self.content.push_str(&chunk.delta);
        if let Some(ref m) = chunk.model {
            self.model = Some(m.clone());
        }
        if let Some(t) = chunk.tokens_in {
            self.tokens_in = t;
        }
        if let Some(t) = chunk.tokens_out {
            self.tokens_out = t;
        }
        if chunk.finish_reason.is_some() {
            self.finish_reason = chunk.finish_reason.clone();
        }
    }

    pub fn finalize(self) -> UnifiedResponse {
        UnifiedResponse {
            content: self.content,
            model: self.model.unwrap_or_default(),
            tokens_in: self.tokens_in,
            tokens_out: self.tokens_out,
            finish_reason: self.finish_reason,
        }
    }
}

/// Parse a single SSE data line into a StreamChunk.
/// Handles OpenAI-format and provider-specific SSE events (`data: {...}`).
pub fn parse_sse_chunk(data: &str, format: &ApiFormat) -> Option<StreamChunk> {
    let data = data.strip_prefix("data: ")?.trim();
    if data == "[DONE]" {
        return None;
    }

    let json: Value = serde_json::from_str(data).ok()?;

    match format {
        ApiFormat::OpenAiCompletions | ApiFormat::OpenAiResponses => {
            let choice = json.get("choices")?.get(0)?;
            let delta = choice.get("delta")?;
            Some(StreamChunk {
                delta: delta
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                model: json
                    .get("model")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                finish_reason: choice
                    .get("finish_reason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                tokens_in: json
                    .get("usage")
                    .and_then(|u| u.get("prompt_tokens"))
                    .and_then(|v| v.as_u64())
                    .map(saturating_u32),
                tokens_out: json
                    .get("usage")
                    .and_then(|u| u.get("completion_tokens"))
                    .and_then(|v| v.as_u64())
                    .map(saturating_u32),
            })
        }
        ApiFormat::AnthropicMessages => {
            let delta = json.get("delta")?;
            Some(StreamChunk {
                delta: delta
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                model: None,
                finish_reason: delta
                    .get("stop_reason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                tokens_in: json
                    .get("usage")
                    .and_then(|u| u.get("input_tokens"))
                    .and_then(|v| v.as_u64())
                    .map(saturating_u32),
                tokens_out: json
                    .get("usage")
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(|v| v.as_u64())
                    .map(saturating_u32),
            })
        }
        ApiFormat::GoogleGenerativeAi => {
            let candidate = json.get("candidates")?.get(0)?;
            let content = candidate.get("content")?;
            let parts = content.get("parts")?.get(0)?;
            Some(StreamChunk {
                delta: parts
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                model: None,
                finish_reason: candidate
                    .get("finishReason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                tokens_in: json
                    .get("usageMetadata")
                    .and_then(|u| u.get("promptTokenCount"))
                    .and_then(|v| v.as_u64())
                    .map(saturating_u32),
                tokens_out: json
                    .get("usageMetadata")
                    .and_then(|u| u.get("candidatesTokenCount"))
                    .and_then(|v| v.as_u64())
                    .map(saturating_u32),
            })
        }
    }
}

pub fn translate_request(request: &UnifiedRequest, format: ApiFormat) -> Result<Value> {
    match format {
        ApiFormat::AnthropicMessages => translate_anthropic(request),
        ApiFormat::OpenAiCompletions => translate_openai_completions(request),
        ApiFormat::OpenAiResponses => translate_openai_responses(request),
        ApiFormat::GoogleGenerativeAi => translate_google(request),
    }
}

fn translate_anthropic(req: &UnifiedRequest) -> Result<Value> {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            let content = match &m.parts {
                Some(parts) if m.is_multimodal() => parts_to_anthropic(parts),
                _ => serde_json::json!(m.content),
            };
            serde_json::json!({
                "role": m.role,
                "content": content,
            })
        })
        .collect();

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": messages,
    });

    if let Some(max) = req.max_tokens {
        body["max_tokens"] = serde_json::json!(max);
    }

    if let Some(ref sys) = req.system {
        body["system"] = serde_json::json!(sys);
    } else {
        let sys_msg = req.messages.iter().find(|m| m.role == "system");
        if let Some(s) = sys_msg {
            body["system"] = serde_json::json!(s.content);
        }
    }

    Ok(body)
}

fn translate_openai_completions(req: &UnifiedRequest) -> Result<Value> {
    let mut messages: Vec<Value> = Vec::new();

    if let Some(ref sys) = req.system {
        messages.push(serde_json::json!({
            "role": "system",
            "content": sys,
        }));
    }

    for m in &req.messages {
        let content = match &m.parts {
            Some(parts) if m.is_multimodal() => parts_to_openai(parts),
            _ => serde_json::json!(m.content),
        };
        messages.push(serde_json::json!({
            "role": m.role,
            "content": content,
        }));
    }

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": messages,
    });

    if let Some(max) = req.max_tokens {
        body["max_tokens"] = serde_json::json!(max);
    }
    if let Some(temp) = req.temperature {
        body["temperature"] = serde_json::json!(temp);
    }

    Ok(body)
}

fn translate_openai_responses(req: &UnifiedRequest) -> Result<Value> {
    let input = req
        .messages
        .iter()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    let mut body = serde_json::json!({
        "model": req.model,
        "input": input,
    });

    if let Some(max) = req.max_tokens {
        body["max_output_tokens"] = serde_json::json!(max);
    }

    Ok(body)
}

fn translate_google(req: &UnifiedRequest) -> Result<Value> {
    let contents: Vec<Value> = req
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            let role = match m.role.as_str() {
                "assistant" => "model",
                other => other,
            };
            serde_json::json!({
                "role": role,
                "parts": [{"text": m.content}],
            })
        })
        .collect();

    let mut gen_config = serde_json::Map::new();
    if let Some(max) = req.max_tokens {
        gen_config.insert("maxOutputTokens".into(), serde_json::json!(max));
    }
    if let Some(temp) = req.temperature {
        gen_config.insert("temperature".into(), serde_json::json!(temp));
    }

    let body = serde_json::json!({
        "contents": contents,
        "generationConfig": gen_config,
    });

    Ok(body)
}

/// Convert multimodal parts to OpenAI-format content blocks.
pub fn parts_to_openai(parts: &[ContentPart]) -> Value {
    let blocks: Vec<Value> = parts
        .iter()
        .map(|p| match p {
            ContentPart::Text { text } => serde_json::json!({
                "type": "text",
                "text": text,
            }),
            ContentPart::ImageUrl { url, detail } => serde_json::json!({
                "type": "image_url",
                "image_url": {
                    "url": url,
                    "detail": detail.as_deref().unwrap_or("auto"),
                }
            }),
            ContentPart::ImageBase64 { media_type, data } => serde_json::json!({
                "type": "image_url",
                "image_url": {
                    "url": format!("data:{media_type};base64,{data}"),
                }
            }),
            ContentPart::AudioTranscription { text, .. } => serde_json::json!({
                "type": "text",
                "text": text,
            }),
        })
        .collect();
    Value::Array(blocks)
}

/// Convert multimodal parts to Anthropic-format content blocks.
pub fn parts_to_anthropic(parts: &[ContentPart]) -> Value {
    let blocks: Vec<Value> = parts
        .iter()
        .map(|p| match p {
            ContentPart::Text { text } => serde_json::json!({
                "type": "text",
                "text": text,
            }),
            ContentPart::ImageUrl { url, .. } => serde_json::json!({
                "type": "image",
                "source": {
                    "type": "url",
                    "url": url,
                }
            }),
            ContentPart::ImageBase64 { media_type, data } => serde_json::json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": media_type,
                    "data": data,
                }
            }),
            ContentPart::AudioTranscription { text, .. } => serde_json::json!({
                "type": "text",
                "text": text,
            }),
        })
        .collect();
    Value::Array(blocks)
}

pub fn translate_response(body: &Value, format: ApiFormat) -> Result<UnifiedResponse> {
    match format {
        ApiFormat::AnthropicMessages => parse_anthropic_response(body),
        ApiFormat::OpenAiCompletions => parse_openai_completions_response(body),
        ApiFormat::OpenAiResponses => parse_openai_responses_response(body),
        ApiFormat::GoogleGenerativeAi => parse_google_response(body),
    }
}

fn parse_anthropic_response(body: &Value) -> Result<UnifiedResponse> {
    let content = body["content"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|block| block["text"].as_str())
        .unwrap_or("")
        .to_string();

    let model = body["model"].as_str().unwrap_or("unknown").to_string();
    let tokens_in = saturating_u32(body["usage"]["input_tokens"].as_u64().unwrap_or(0));
    let tokens_out = saturating_u32(body["usage"]["output_tokens"].as_u64().unwrap_or(0));
    let finish_reason = body["stop_reason"].as_str().map(String::from);

    Ok(UnifiedResponse {
        content,
        model,
        tokens_in,
        tokens_out,
        finish_reason,
    })
}

fn parse_openai_completions_response(body: &Value) -> Result<UnifiedResponse> {
    let choice = body["choices"]
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or_else(|| IroncladError::Llm("no choices in OpenAI response".into()))?;

    let content = choice["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let model = body["model"].as_str().unwrap_or("unknown").to_string();
    let tokens_in = saturating_u32(body["usage"]["prompt_tokens"].as_u64().unwrap_or(0));
    let tokens_out = saturating_u32(body["usage"]["completion_tokens"].as_u64().unwrap_or(0));
    let finish_reason = choice["finish_reason"].as_str().map(String::from);

    Ok(UnifiedResponse {
        content,
        model,
        tokens_in,
        tokens_out,
        finish_reason,
    })
}

fn parse_openai_responses_response(body: &Value) -> Result<UnifiedResponse> {
    let content = body["output"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|item| item["content"].as_array())
        .and_then(|parts| parts.first())
        .and_then(|part| part["text"].as_str())
        .unwrap_or("")
        .to_string();

    let model = body["model"].as_str().unwrap_or("unknown").to_string();
    let tokens_in = saturating_u32(body["usage"]["input_tokens"].as_u64().unwrap_or(0));
    let tokens_out = saturating_u32(body["usage"]["output_tokens"].as_u64().unwrap_or(0));

    Ok(UnifiedResponse {
        content,
        model,
        tokens_in,
        tokens_out,
        finish_reason: None,
    })
}

fn parse_google_response(body: &Value) -> Result<UnifiedResponse> {
    let content = body["candidates"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|c| c["content"]["parts"].as_array())
        .and_then(|parts| parts.first())
        .and_then(|p| p["text"].as_str())
        .unwrap_or("")
        .to_string();

    let model = body["modelVersion"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let tokens_in = saturating_u32(
        body["usageMetadata"]["promptTokenCount"]
            .as_u64()
            .unwrap_or(0),
    );
    let tokens_out = saturating_u32(
        body["usageMetadata"]["candidatesTokenCount"]
            .as_u64()
            .unwrap_or(0),
    );
    let finish_reason = body["candidates"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|c| c["finishReason"].as_str())
        .map(String::from);

    Ok(UnifiedResponse {
        content,
        model,
        tokens_in,
        tokens_out,
        finish_reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> UnifiedRequest {
        UnifiedRequest {
            model: "claude-sonnet-4-20250514".into(),
            messages: vec![
                UnifiedMessage {
                    role: "user".into(),
                    content: "Hello".into(),
                    parts: None,
                },
                UnifiedMessage {
                    role: "assistant".into(),
                    content: "Hi there".into(),
                    parts: None,
                },
                UnifiedMessage {
                    role: "user".into(),
                    content: "How are you?".into(),
                    parts: None,
                },
            ],
            max_tokens: Some(1024),
            temperature: Some(0.7),
            system: Some("You are helpful.".into()),
            quality_target: None,
        }
    }

    #[test]
    fn translate_request_anthropic() {
        let req = sample_request();
        let body = translate_request(&req, ApiFormat::AnthropicMessages).unwrap();

        assert_eq!(body["model"], "claude-sonnet-4-20250514");
        assert_eq!(body["system"], "You are helpful.");
        assert_eq!(body["max_tokens"], 1024);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["role"], "user");
    }

    #[test]
    fn translate_request_openai_completions() {
        let req = sample_request();
        let body = translate_request(&req, ApiFormat::OpenAiCompletions).unwrap();

        assert_eq!(body["model"], "claude-sonnet-4-20250514");
        assert_eq!(body["temperature"], 0.7);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "You are helpful.");
        assert_eq!(msgs.len(), 4); // system + 3 conversation
    }

    #[test]
    fn translate_request_openai_responses() {
        let req = sample_request();
        let body = translate_request(&req, ApiFormat::OpenAiResponses).unwrap();

        assert_eq!(body["model"], "claude-sonnet-4-20250514");
        assert!(body["input"].as_str().unwrap().contains("Hello"));
        assert_eq!(body["max_output_tokens"], 1024);
    }

    #[test]
    fn translate_request_google() {
        let req = sample_request();
        let body = translate_request(&req, ApiFormat::GoogleGenerativeAi).unwrap();

        let contents = body["contents"].as_array().unwrap();
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[1]["role"], "model"); // assistant -> model
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 1024);
        assert_eq!(body["generationConfig"]["temperature"], 0.7);
    }

    #[test]
    fn translate_response_anthropic() {
        let body = serde_json::json!({
            "content": [{"type": "text", "text": "Hello from Claude"}],
            "model": "claude-sonnet-4-20250514",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });

        let resp = translate_response(&body, ApiFormat::AnthropicMessages).unwrap();
        assert_eq!(resp.content, "Hello from Claude");
        assert_eq!(resp.model, "claude-sonnet-4-20250514");
        assert_eq!(resp.tokens_in, 10);
        assert_eq!(resp.tokens_out, 5);
        assert_eq!(resp.finish_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn translate_response_openai_completions() {
        let body = serde_json::json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Hello from GPT"},
                "finish_reason": "stop"
            }],
            "model": "gpt-4o",
            "usage": {"prompt_tokens": 12, "completion_tokens": 8}
        });

        let resp = translate_response(&body, ApiFormat::OpenAiCompletions).unwrap();
        assert_eq!(resp.content, "Hello from GPT");
        assert_eq!(resp.tokens_in, 12);
        assert_eq!(resp.tokens_out, 8);
        assert_eq!(resp.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn translate_response_openai_responses() {
        let body = serde_json::json!({
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "Hello from Responses API"}]
            }],
            "model": "gpt-4o",
            "usage": {"input_tokens": 15, "output_tokens": 10}
        });

        let resp = translate_response(&body, ApiFormat::OpenAiResponses).unwrap();
        assert_eq!(resp.content, "Hello from Responses API");
        assert_eq!(resp.tokens_in, 15);
        assert_eq!(resp.tokens_out, 10);
    }

    #[test]
    fn translate_response_google() {
        let body = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello from Gemini"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "modelVersion": "gemini-2.5-flash",
            "usageMetadata": {"promptTokenCount": 20, "candidatesTokenCount": 6}
        });

        let resp = translate_response(&body, ApiFormat::GoogleGenerativeAi).unwrap();
        assert_eq!(resp.content, "Hello from Gemini");
        assert_eq!(resp.model, "gemini-2.5-flash");
        assert_eq!(resp.tokens_in, 20);
        assert_eq!(resp.tokens_out, 6);
        assert_eq!(resp.finish_reason.as_deref(), Some("STOP"));
    }

    #[test]
    fn stream_accumulator_empty() {
        let acc = StreamAccumulator::default();
        let resp = acc.finalize();
        assert_eq!(resp.content, "");
        assert_eq!(resp.model, "");
        assert_eq!(resp.tokens_in, 0);
        assert_eq!(resp.tokens_out, 0);
        assert!(resp.finish_reason.is_none());
    }

    #[test]
    fn stream_accumulator_pushes_deltas() {
        let mut acc = StreamAccumulator::default();
        for text in ["Hello", ", ", "world!"] {
            acc.push(&StreamChunk {
                delta: text.into(),
                model: None,
                finish_reason: None,
                tokens_in: None,
                tokens_out: None,
            });
        }
        let resp = acc.finalize();
        assert_eq!(resp.content, "Hello, world!");
    }

    #[test]
    fn stream_accumulator_captures_model() {
        let mut acc = StreamAccumulator::default();
        acc.push(&StreamChunk {
            delta: "hi".into(),
            model: Some("gpt-4o".into()),
            finish_reason: None,
            tokens_in: None,
            tokens_out: None,
        });
        let resp = acc.finalize();
        assert_eq!(resp.model, "gpt-4o");
    }

    #[test]
    fn stream_accumulator_captures_tokens_from_last() {
        let mut acc = StreamAccumulator::default();
        acc.push(&StreamChunk {
            delta: "a".into(),
            model: None,
            finish_reason: None,
            tokens_in: Some(5),
            tokens_out: Some(1),
        });
        acc.push(&StreamChunk {
            delta: "b".into(),
            model: None,
            finish_reason: Some("stop".into()),
            tokens_in: Some(10),
            tokens_out: Some(2),
        });
        let resp = acc.finalize();
        assert_eq!(resp.tokens_in, 10);
        assert_eq!(resp.tokens_out, 2);
        assert_eq!(resp.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn parse_sse_done_returns_none() {
        let result = parse_sse_chunk("data: [DONE]", &ApiFormat::OpenAiCompletions);
        assert!(result.is_none());
    }

    #[test]
    fn parse_sse_openai_chunk() {
        let line = r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}],"model":"gpt-4o","usage":{"prompt_tokens":12,"completion_tokens":1}}"#;
        let chunk = parse_sse_chunk(line, &ApiFormat::OpenAiCompletions).unwrap();
        assert_eq!(chunk.delta, "Hi");
        assert_eq!(chunk.model.as_deref(), Some("gpt-4o"));
        assert!(chunk.finish_reason.is_none());
        assert_eq!(chunk.tokens_in, Some(12));
        assert_eq!(chunk.tokens_out, Some(1));
    }

    #[test]
    fn parse_sse_non_data_line_returns_none() {
        let result = parse_sse_chunk("event: message", &ApiFormat::OpenAiCompletions);
        assert!(result.is_none());
    }

    #[test]
    fn content_part_text_to_text() {
        let part = ContentPart::text("hello");
        assert_eq!(part.to_text(), "hello");
    }

    #[test]
    fn content_part_image_url_to_text() {
        let part = ContentPart::image_url("https://example.com/img.png");
        assert_eq!(part.to_text(), "[Image: https://example.com/img.png]");
    }

    #[test]
    fn content_part_audio_to_text() {
        let part = ContentPart::audio_transcription("Hello world", "whatsapp");
        assert_eq!(part.to_text(), "[Audio from whatsapp]: Hello world");
    }

    #[test]
    fn unified_message_text_helper() {
        let msg = UnifiedMessage::text("user", "hi there");
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content, "hi there");
        assert!(msg.parts.is_none());
    }

    #[test]
    fn unified_message_multimodal_helper() {
        let msg = UnifiedMessage::multimodal(
            "user",
            vec![
                ContentPart::text("Look at this:"),
                ContentPart::image_url("https://example.com/photo.jpg"),
            ],
        );
        assert_eq!(msg.role, "user");
        assert!(msg.parts.is_some());
        assert_eq!(msg.parts.as_ref().unwrap().len(), 2);
        assert!(msg.content.contains("Look at this:"));
        assert!(
            msg.content
                .contains("[Image: https://example.com/photo.jpg]")
        );
    }

    #[test]
    fn unified_message_is_multimodal_false_for_text() {
        let msg = UnifiedMessage::text("user", "plain text");
        assert!(!msg.is_multimodal());
    }

    #[test]
    fn unified_message_is_multimodal_true_with_image() {
        let msg = UnifiedMessage::multimodal(
            "user",
            vec![
                ContentPart::text("describe this"),
                ContentPart::image_url("https://example.com/cat.jpg"),
            ],
        );
        assert!(msg.is_multimodal());
    }

    #[test]
    fn parts_to_openai_text_only() {
        let parts = vec![ContentPart::text("hello")];
        let result = parts_to_openai(&parts);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "hello");
    }

    #[test]
    fn parts_to_openai_with_image() {
        let parts = vec![
            ContentPart::text("What is in this image?"),
            ContentPart::image_url("https://example.com/img.png"),
        ];
        let result = parts_to_openai(&parts);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[1]["type"], "image_url");
        assert_eq!(arr[1]["image_url"]["url"], "https://example.com/img.png");
        assert_eq!(arr[1]["image_url"]["detail"], "auto");
    }

    #[test]
    fn parts_to_anthropic_base64_image() {
        let parts = vec![ContentPart::image_base64("image/png", "iVBOR...")];
        let result = parts_to_anthropic(&parts);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "image");
        assert_eq!(arr[0]["source"]["type"], "base64");
        assert_eq!(arr[0]["source"]["media_type"], "image/png");
        assert_eq!(arr[0]["source"]["data"], "iVBOR...");
    }

    // ── SSE Anthropic parsing ──────────────────────────────

    #[test]
    fn parse_sse_anthropic_chunk() {
        let line = r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello"},"usage":{"input_tokens":10,"output_tokens":3}}"#;
        let chunk = parse_sse_chunk(line, &ApiFormat::AnthropicMessages).unwrap();
        assert_eq!(chunk.delta, "Hello");
        assert!(chunk.model.is_none());
        assert_eq!(chunk.tokens_in, Some(10));
        assert_eq!(chunk.tokens_out, Some(3));
    }

    #[test]
    fn parse_sse_anthropic_with_stop_reason() {
        let line = r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"","stop_reason":"end_turn"}}"#;
        let chunk = parse_sse_chunk(line, &ApiFormat::AnthropicMessages).unwrap();
        assert_eq!(chunk.delta, "");
        assert_eq!(chunk.finish_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn parse_sse_anthropic_no_text_returns_empty() {
        let line = r#"data: {"type":"content_block_delta","delta":{"type":"text_delta"}}"#;
        let chunk = parse_sse_chunk(line, &ApiFormat::AnthropicMessages).unwrap();
        assert_eq!(chunk.delta, "");
    }

    #[test]
    fn parse_sse_anthropic_missing_delta_returns_none() {
        let line = r#"data: {"type":"ping"}"#;
        let result = parse_sse_chunk(line, &ApiFormat::AnthropicMessages);
        assert!(result.is_none());
    }

    // ── SSE Google parsing ──────────────────────────────

    #[test]
    fn parse_sse_google_chunk() {
        let line = r#"data: {"candidates":[{"content":{"parts":[{"text":"World"}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":2}}"#;
        let chunk = parse_sse_chunk(line, &ApiFormat::GoogleGenerativeAi).unwrap();
        assert_eq!(chunk.delta, "World");
        assert_eq!(chunk.finish_reason.as_deref(), Some("STOP"));
        assert_eq!(chunk.tokens_in, Some(5));
        assert_eq!(chunk.tokens_out, Some(2));
        assert!(chunk.model.is_none());
    }

    #[test]
    fn parse_sse_google_no_text_returns_empty() {
        let line = r#"data: {"candidates":[{"content":{"parts":[{}],"role":"model"}}]}"#;
        let chunk = parse_sse_chunk(line, &ApiFormat::GoogleGenerativeAi).unwrap();
        assert_eq!(chunk.delta, "");
    }

    #[test]
    fn parse_sse_google_missing_candidates_returns_none() {
        let line = r#"data: {"error":"something"}"#;
        assert!(parse_sse_chunk(line, &ApiFormat::GoogleGenerativeAi).is_none());
    }

    // ── SSE edge cases ──────────────────────────────

    #[test]
    fn parse_sse_invalid_json_returns_none() {
        let line = "data: {not-json}";
        assert!(parse_sse_chunk(line, &ApiFormat::OpenAiCompletions).is_none());
    }

    #[test]
    fn parse_sse_done_for_all_formats() {
        assert!(parse_sse_chunk("data: [DONE]", &ApiFormat::AnthropicMessages).is_none());
        assert!(parse_sse_chunk("data: [DONE]", &ApiFormat::GoogleGenerativeAi).is_none());
        assert!(parse_sse_chunk("data: [DONE]", &ApiFormat::OpenAiResponses).is_none());
    }

    #[test]
    fn parse_sse_openai_with_finish_reason() {
        let line = r#"data: {"id":"x","choices":[{"index":0,"delta":{"content":"bye"},"finish_reason":"stop"}],"model":"gpt-4o"}"#;
        let chunk = parse_sse_chunk(line, &ApiFormat::OpenAiCompletions).unwrap();
        assert_eq!(chunk.delta, "bye");
        assert_eq!(chunk.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn parse_sse_openai_missing_content_returns_empty_delta() {
        let line = r#"data: {"id":"x","choices":[{"index":0,"delta":{},"finish_reason":null}]}"#;
        let chunk = parse_sse_chunk(line, &ApiFormat::OpenAiCompletions).unwrap();
        assert_eq!(chunk.delta, "");
    }

    #[test]
    fn parse_sse_openai_null_content_returns_empty_delta() {
        let line = r#"data: {"id":"x","choices":[{"index":0,"delta":{"content":null},"finish_reason":null}]}"#;
        let chunk = parse_sse_chunk(line, &ApiFormat::OpenAiCompletions).unwrap();
        assert_eq!(chunk.delta, "");
    }

    #[test]
    fn parse_sse_openai_responses_format() {
        // OpenAiResponses format is the same as OpenAiCompletions for SSE
        let line = r#"data: {"id":"x","choices":[{"index":0,"delta":{"content":"test"},"finish_reason":null}],"model":"gpt-4o"}"#;
        let chunk = parse_sse_chunk(line, &ApiFormat::OpenAiResponses).unwrap();
        assert_eq!(chunk.delta, "test");
    }

    // ── multimodal translation: Anthropic ──────────────

    #[test]
    fn translate_anthropic_multimodal_message() {
        let req = UnifiedRequest {
            model: "claude-sonnet-4-20250514".into(),
            messages: vec![UnifiedMessage::multimodal(
                "user",
                vec![
                    ContentPart::text("What is this?"),
                    ContentPart::image_url("https://example.com/img.png"),
                    ContentPart::image_base64("image/jpeg", "abc123"),
                ],
            )],
            max_tokens: Some(512),
            temperature: None,
            system: None,
            quality_target: None,
        };
        let body = translate_request(&req, ApiFormat::AnthropicMessages).unwrap();
        let msgs = body["messages"].as_array().unwrap();
        let content = msgs[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 3);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "url");
        assert_eq!(content[2]["type"], "image");
        assert_eq!(content[2]["source"]["type"], "base64");
    }

    #[test]
    fn translate_anthropic_system_from_message_when_no_system_field() {
        let req = UnifiedRequest {
            model: "claude-sonnet-4-20250514".into(),
            messages: vec![
                UnifiedMessage::text("system", "Be concise."),
                UnifiedMessage::text("user", "Hello"),
            ],
            max_tokens: None,
            temperature: None,
            system: None, // no explicit system field
            quality_target: None,
        };
        let body = translate_request(&req, ApiFormat::AnthropicMessages).unwrap();
        assert_eq!(body["system"], "Be concise.");
        // system message should be filtered from messages array
        let msgs = body["messages"].as_array().unwrap();
        assert!(msgs.iter().all(|m| m["role"] != "system"));
    }

    #[test]
    fn translate_anthropic_no_max_tokens() {
        let req = UnifiedRequest {
            model: "claude-sonnet-4-20250514".into(),
            messages: vec![UnifiedMessage::text("user", "Hi")],
            max_tokens: None,
            temperature: None,
            system: None,
            quality_target: None,
        };
        let body = translate_request(&req, ApiFormat::AnthropicMessages).unwrap();
        assert!(body.get("max_tokens").is_none() || body["max_tokens"].is_null());
    }

    // ── multimodal translation: OpenAI ──────────────

    #[test]
    fn translate_openai_multimodal_message() {
        let req = UnifiedRequest {
            model: "gpt-4o".into(),
            messages: vec![UnifiedMessage::multimodal(
                "user",
                vec![
                    ContentPart::text("Describe this"),
                    ContentPart::image_url("https://example.com/photo.jpg"),
                ],
            )],
            max_tokens: Some(1000),
            temperature: Some(0.5),
            system: Some("You are an image analyzer".into()),
            quality_target: None,
        };
        let body = translate_request(&req, ApiFormat::OpenAiCompletions).unwrap();
        let msgs = body["messages"].as_array().unwrap();
        // First message is system
        assert_eq!(msgs[0]["role"], "system");
        // Second message has multimodal content
        let content = msgs[1]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
    }

    #[test]
    fn translate_openai_no_system_no_max_tokens_no_temp() {
        let req = UnifiedRequest {
            model: "gpt-4o-mini".into(),
            messages: vec![UnifiedMessage::text("user", "Hi")],
            max_tokens: None,
            temperature: None,
            system: None,
            quality_target: None,
        };
        let body = translate_request(&req, ApiFormat::OpenAiCompletions).unwrap();
        assert!(body.get("max_tokens").is_none() || body["max_tokens"].is_null());
        assert!(body.get("temperature").is_none() || body["temperature"].is_null());
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1); // no system message
    }

    // ── OpenAI Responses format ──────────────────────

    #[test]
    fn translate_openai_responses_no_max_tokens() {
        let req = UnifiedRequest {
            model: "gpt-4o".into(),
            messages: vec![UnifiedMessage::text("user", "Hello")],
            max_tokens: None,
            temperature: None,
            system: None,
            quality_target: None,
        };
        let body = translate_request(&req, ApiFormat::OpenAiResponses).unwrap();
        assert!(body.get("max_output_tokens").is_none() || body["max_output_tokens"].is_null());
    }

    // ── Google format ──────────────────────

    #[test]
    fn translate_google_no_gen_config_fields() {
        let req = UnifiedRequest {
            model: "gemini-2.5-flash".into(),
            messages: vec![UnifiedMessage::text("user", "Hello")],
            max_tokens: None,
            temperature: None,
            system: None,
            quality_target: None,
        };
        let body = translate_request(&req, ApiFormat::GoogleGenerativeAi).unwrap();
        let gen_cfg = body["generationConfig"].as_object().unwrap();
        assert!(gen_cfg.is_empty(), "no gen config fields if none set");
    }

    #[test]
    fn translate_google_filters_system_messages() {
        let req = UnifiedRequest {
            model: "gemini-2.5-flash".into(),
            messages: vec![
                UnifiedMessage::text("system", "Be helpful"),
                UnifiedMessage::text("user", "Hello"),
            ],
            max_tokens: None,
            temperature: None,
            system: None,
            quality_target: None,
        };
        let body = translate_request(&req, ApiFormat::GoogleGenerativeAi).unwrap();
        let contents = body["contents"].as_array().unwrap();
        // system message should be filtered
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
    }

    // ── Response parsing edge cases ──────────────────

    #[test]
    fn parse_anthropic_response_empty_content() {
        let body = serde_json::json!({
            "content": [],
            "model": "claude-sonnet-4-20250514",
            "usage": {"input_tokens": 0, "output_tokens": 0}
        });
        let resp = translate_response(&body, ApiFormat::AnthropicMessages).unwrap();
        assert_eq!(resp.content, "");
    }

    #[test]
    fn parse_openai_response_no_choices_errors() {
        let body = serde_json::json!({"choices": []});
        let err = translate_response(&body, ApiFormat::OpenAiCompletions);
        assert!(err.is_err());
    }

    #[test]
    fn parse_openai_responses_empty_output() {
        let body = serde_json::json!({
            "output": [],
            "model": "gpt-4o",
            "usage": {"input_tokens": 0, "output_tokens": 0}
        });
        let resp = translate_response(&body, ApiFormat::OpenAiResponses).unwrap();
        assert_eq!(resp.content, "");
    }

    #[test]
    fn parse_google_response_no_candidates() {
        let body = serde_json::json!({
            "candidates": [],
            "modelVersion": "gemini-2.5-flash"
        });
        let resp = translate_response(&body, ApiFormat::GoogleGenerativeAi).unwrap();
        assert_eq!(resp.content, "");
    }

    #[test]
    fn parse_anthropic_response_missing_usage() {
        let body = serde_json::json!({
            "content": [{"type": "text", "text": "Hello"}],
            "model": "claude-sonnet-4-20250514"
        });
        let resp = translate_response(&body, ApiFormat::AnthropicMessages).unwrap();
        assert_eq!(resp.tokens_in, 0);
        assert_eq!(resp.tokens_out, 0);
    }

    // ── parts_to_openai edge cases ──────────────────────

    #[test]
    fn parts_to_openai_base64_image() {
        let parts = vec![ContentPart::image_base64("image/png", "base64data")];
        let result = parts_to_openai(&parts);
        let arr = result.as_array().unwrap();
        assert_eq!(arr[0]["type"], "image_url");
        let url = arr[0]["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
        assert!(url.contains("base64data"));
    }

    #[test]
    fn parts_to_openai_audio_becomes_text() {
        let parts = vec![ContentPart::audio_transcription("Hello world", "whisper")];
        let result = parts_to_openai(&parts);
        let arr = result.as_array().unwrap();
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "Hello world");
    }

    #[test]
    fn parts_to_anthropic_url_image() {
        let parts = vec![ContentPart::image_url("https://example.com/photo.jpg")];
        let result = parts_to_anthropic(&parts);
        let arr = result.as_array().unwrap();
        assert_eq!(arr[0]["type"], "image");
        assert_eq!(arr[0]["source"]["type"], "url");
        assert_eq!(arr[0]["source"]["url"], "https://example.com/photo.jpg");
    }

    #[test]
    fn parts_to_anthropic_audio_becomes_text() {
        let parts = vec![ContentPart::audio_transcription("Transcript", "microphone")];
        let result = parts_to_anthropic(&parts);
        let arr = result.as_array().unwrap();
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "Transcript");
    }

    // ── ContentPart constructors ──────────────────────

    #[test]
    fn content_part_image_base64_to_text() {
        let part = ContentPart::image_base64("image/webp", "data123");
        assert_eq!(part.to_text(), "[Image: image/webp]");
    }

    // ── is_multimodal with only text parts ──────────────────

    #[test]
    fn is_multimodal_false_with_text_only_parts() {
        let msg = UnifiedMessage {
            role: "user".into(),
            content: "hello".into(),
            parts: Some(vec![ContentPart::text("hello")]),
        };
        // parts exist but are all Text -> not multimodal
        assert!(!msg.is_multimodal());
    }

    // ── UnifiedRequest serde ──────────────────────

    #[test]
    fn unified_request_serialization_roundtrip() {
        let req = UnifiedRequest {
            model: "gpt-4o".into(),
            messages: vec![UnifiedMessage::text("user", "hello")],
            max_tokens: Some(100),
            temperature: Some(0.5),
            system: Some("sys".into()),
            quality_target: Some(0.9),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: UnifiedRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.model, "gpt-4o");
        assert_eq!(parsed.quality_target, Some(0.9));
    }

    #[test]
    fn unified_response_serialization_roundtrip() {
        let resp = UnifiedResponse {
            content: "Hello".into(),
            model: "gpt-4o".into(),
            tokens_in: 10,
            tokens_out: 5,
            finish_reason: Some("stop".into()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: UnifiedResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.content, "Hello");
        assert_eq!(parsed.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn stream_chunk_serialization_roundtrip() {
        let chunk = StreamChunk {
            delta: "hi".into(),
            model: Some("gpt-4o".into()),
            finish_reason: None,
            tokens_in: Some(5),
            tokens_out: None,
        };
        let json = serde_json::to_string(&chunk).unwrap();
        let parsed: StreamChunk = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.delta, "hi");
        assert_eq!(parsed.tokens_in, Some(5));
        assert!(parsed.tokens_out.is_none());
    }

    // ── mixed multimodal parts ──────────────────────

    #[test]
    fn parts_to_openai_mixed_all_types() {
        let parts = vec![
            ContentPart::text("Look:"),
            ContentPart::image_url("https://a.com/img.png"),
            ContentPart::image_base64("image/gif", "R0lGOD"),
            ContentPart::audio_transcription("speech", "mic"),
        ];
        let result = parts_to_openai(&parts);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[1]["type"], "image_url");
        assert_eq!(arr[2]["type"], "image_url");
        assert_eq!(arr[3]["type"], "text");
    }

    #[test]
    fn parts_to_anthropic_mixed_all_types() {
        let parts = vec![
            ContentPart::text("Look:"),
            ContentPart::image_url("https://a.com/img.png"),
            ContentPart::image_base64("image/gif", "R0lGOD"),
            ContentPart::audio_transcription("speech", "mic"),
        ];
        let result = parts_to_anthropic(&parts);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[1]["type"], "image");
        assert_eq!(arr[2]["type"], "image");
        assert_eq!(arr[3]["type"], "text");
    }

    #[test]
    fn image_url_detail_passed_through_openai() {
        let part = ContentPart::ImageUrl {
            url: "https://a.com/img.png".into(),
            detail: Some("high".into()),
        };
        let result = parts_to_openai(&[part]);
        let arr = result.as_array().unwrap();
        assert_eq!(arr[0]["image_url"]["detail"], "high");
    }
}
