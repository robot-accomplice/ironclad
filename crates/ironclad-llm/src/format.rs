use serde::{Deserialize, Serialize};
use serde_json::Value;

use ironclad_core::{ApiFormat, IroncladError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedRequest {
    pub model: String,
    pub messages: Vec<UnifiedMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub system: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnifiedMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedResponse {
    pub content: String,
    pub model: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub finish_reason: Option<String>,
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
            serde_json::json!({
                "role": m.role,
                "content": m.content,
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
        messages.push(serde_json::json!({
            "role": m.role,
            "content": m.content,
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
    let tokens_in = body["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
    let tokens_out = body["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;
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
    let tokens_in = body["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32;
    let tokens_out = body["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32;
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
    let tokens_in = body["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
    let tokens_out = body["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;

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
    let tokens_in = body["usageMetadata"]["promptTokenCount"]
        .as_u64()
        .unwrap_or(0) as u32;
    let tokens_out = body["usageMetadata"]["candidatesTokenCount"]
        .as_u64()
        .unwrap_or(0) as u32;
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
                },
                UnifiedMessage {
                    role: "assistant".into(),
                    content: "Hi there".into(),
                },
                UnifiedMessage {
                    role: "user".into(),
                    content: "How are you?".into(),
                },
            ],
            max_tokens: Some(1024),
            temperature: Some(0.7),
            system: Some("You are helpful.".into()),
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
}
