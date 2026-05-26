use std::io::{BufRead, BufReader};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::Serialize;

use super::{
    openai_responses::{
        openai_responses_tools, parse_openai_responses_stream_line, OpenAiResponsesTool,
    },
    stream::{
        apply_model_headers, emit_usage_if_present, merge_usage, resolve_api_key, ProviderError,
        StreamDelta, ToolCallAccumulator,
    },
};
use crate::{
    llm_convert::openai_responses_input, AssistantMessage, ProviderAdapter, ProviderEvent,
    ProviderRequest, TokenUsage,
};

const DEFAULT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";
const JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";

#[derive(Clone, Debug, Default)]
pub struct OpenAiCodexResponsesProvider;

impl ProviderAdapter for OpenAiCodexResponsesProvider {
    fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        if let Err(error) = openai_codex_responses_stream(request, emit) {
            emit(ProviderEvent::Error(error.to_string()));
        }
    }
}

#[derive(Debug, Serialize)]
struct OpenAiCodexResponsesRequest {
    model: String,
    input: Vec<serde_json::Value>,
    stream: bool,
    store: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OpenAiResponsesTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
}

fn openai_codex_responses_stream(
    request: ProviderRequest,
    emit: &mut dyn FnMut(ProviderEvent),
) -> Result<(), ProviderError> {
    let api_key = resolve_api_key(&request.model)?;
    let account_id = extract_chatgpt_account_id(&api_key)?;
    let url = resolve_codex_url(request.model.base_url.as_deref());
    let tools = openai_responses_tools(&request.tools);
    let tool_choice = (!tools.is_empty()).then_some("auto");
    let parallel_tool_calls = (!tools.is_empty()).then_some(true);
    let body = OpenAiCodexResponsesRequest {
        model: request.model.id.clone(),
        input: openai_responses_input(&request.messages),
        stream: true,
        store: false,
        tools,
        tool_choice,
        parallel_tool_calls,
    };

    let request_builder = reqwest::blocking::Client::new()
        .post(url)
        .bearer_auth(api_key)
        .header("chatgpt-account-id", account_id)
        .header("originator", "exgent")
        .header(reqwest::header::USER_AGENT, "exgent")
        .header("OpenAI-Beta", "responses=experimental")
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .header(reqwest::header::CONTENT_TYPE, "application/json");
    let request_builder = apply_model_headers(request_builder, &request.model, &request.messages);

    let response = request_builder
        .json(&body)
        .send()
        .map_err(|error| ProviderError::new(format!("request failed: {error}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .unwrap_or_else(|error| format!("failed to read error body: {error}"));
        return Err(ProviderError::new(format!(
            "provider returned {status}: {body}"
        )));
    }

    let model = request.model;
    let mut content = String::new();
    let mut usage = TokenUsage::default();
    let reader = BufReader::new(response);
    emit(ProviderEvent::Start);

    for line in reader.lines() {
        let line =
            line.map_err(|error| ProviderError::new(format!("stream read failed: {error}")))?;
        for delta in parse_openai_codex_stream_line(&line)? {
            match delta {
                StreamDelta::Text(delta) => {
                    content.push_str(&delta);
                    emit(ProviderEvent::TextDelta(delta));
                }
                StreamDelta::Reasoning(delta) => emit(ProviderEvent::ReasoningDelta(delta)),
                StreamDelta::Usage(delta_usage) => merge_usage(&mut usage, delta_usage),
                StreamDelta::ToolCall(call) => emit(ProviderEvent::ToolCall(call)),
                StreamDelta::OpenAiToolCallDelta(delta) => {
                    let mut tool_calls = ToolCallAccumulator::default();
                    tool_calls.push_openai_delta(delta);
                    for call in tool_calls.finish()? {
                        emit(ProviderEvent::ToolCall(call));
                    }
                }
            }
        }
    }

    emit_usage_if_present(&usage, emit);
    emit(ProviderEvent::Done(Box::new(AssistantMessage {
        model,
        content,
    })));
    Ok(())
}

fn parse_openai_codex_stream_line(line: &str) -> Result<Vec<StreamDelta>, ProviderError> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') {
        return Ok(Vec::new());
    }

    let Some(data) = line.strip_prefix("data:") else {
        return Ok(Vec::new());
    };
    let data = data.trim();
    if data == "[DONE]" {
        return Ok(Vec::new());
    }

    let mut value: serde_json::Value = serde_json::from_str(data)
        .map_err(|error| ProviderError::new(format!("invalid stream chunk: {error}")))?;
    if value.get("type").and_then(serde_json::Value::as_str) == Some("response.done")
        || value.get("type").and_then(serde_json::Value::as_str) == Some("response.incomplete")
    {
        value["type"] = serde_json::Value::String("response.completed".to_string());
    }

    parse_openai_responses_stream_line(&format!("data: {value}"))
}

fn resolve_codex_url(base_url: Option<&str>) -> String {
    let raw = base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_CODEX_BASE_URL);
    let normalized = raw.trim_end_matches('/');
    if normalized.ends_with("/codex/responses") {
        normalized.to_string()
    } else if normalized.ends_with("/codex") {
        format!("{normalized}/responses")
    } else {
        format!("{normalized}/codex/responses")
    }
}

fn extract_chatgpt_account_id(token: &str) -> Result<String, ProviderError> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or_else(|| ProviderError::new("OpenAI Codex token must be a JWT access token"))?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|error| ProviderError::new(format!("invalid OpenAI Codex token: {error}")))?;
    let value: serde_json::Value = serde_json::from_slice(&decoded).map_err(|error| {
        ProviderError::new(format!("invalid OpenAI Codex token payload: {error}"))
    })?;
    value
        .get(JWT_CLAIM_PATH)
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|account_id| !account_id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ProviderError::new("OpenAI Codex token missing chatgpt_account_id"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde_json::json;

    #[test]
    fn resolves_codex_responses_url() {
        assert_eq!(
            resolve_codex_url(None),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            resolve_codex_url(Some("https://example.test/backend-api/codex")),
            "https://example.test/backend-api/codex/responses"
        );
        assert_eq!(
            resolve_codex_url(Some("https://example.test/backend-api/codex/responses")),
            "https://example.test/backend-api/codex/responses"
        );
    }

    #[test]
    fn extracts_account_id_from_codex_jwt() {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            json!({
                JWT_CLAIM_PATH: {
                    "chatgpt_account_id": "acct_123"
                }
            })
            .to_string(),
        );
        let token = format!("{header}.{payload}.sig");

        assert_eq!(extract_chatgpt_account_id(&token).unwrap(), "acct_123");
    }

    #[test]
    fn normalizes_codex_done_event_to_completed_usage() {
        let deltas = parse_openai_codex_stream_line(
            r#"data: {"type":"response.done","response":{"usage":{"input_tokens":3,"output_tokens":2}}}"#,
        )
        .unwrap();

        assert_eq!(
            deltas,
            vec![StreamDelta::Usage(TokenUsage {
                input: 3,
                output: 2,
                cache_read: 0,
                cache_write: 0,
            })]
        );
    }
}
