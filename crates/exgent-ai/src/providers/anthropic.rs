use std::io::{BufRead, BufReader};

use serde::Serialize;

use super::stream::{
    apply_model_headers, emit_usage_if_present, is_empty_json_object, json_usize, merge_usage,
    resolve_api_key, AnthropicToolStreamDelta, ProviderError, StreamDelta, ToolCallAccumulator,
};
use crate::{
    llm_convert::{anthropic_messages, AnthropicMessage},
    AssistantMessage, ChatMessage, MessageRole, ProviderAdapter, ProviderEvent, ProviderRequest,
    TokenUsage, ToolDefinition,
};

#[derive(Clone, Debug, Default)]
pub struct AnthropicMessagesProvider;

impl ProviderAdapter for AnthropicMessagesProvider {
    fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        self.stream_events_cancellable(request, &|| false, emit);
    }

    fn stream_events_cancellable(
        &self,
        request: ProviderRequest,
        should_cancel: &dyn Fn() -> bool,
        emit: &mut dyn FnMut(ProviderEvent),
    ) {
        if let Err(error) = anthropic_messages_stream(request, should_cancel, emit) {
            emit(ProviderEvent::Error(error.to_string()));
        }
    }
}

#[derive(Debug, Serialize)]
struct AnthropicMessagesRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool>,
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

fn anthropic_messages_stream(
    request: ProviderRequest,
    should_cancel: &dyn Fn() -> bool,
    emit: &mut dyn FnMut(ProviderEvent),
) -> Result<(), ProviderError> {
    if should_cancel() {
        return Err(ProviderError::new("cancelled"));
    }

    let base_url = request
        .model
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.anthropic.com".to_string());
    let api_key = resolve_api_key(&request.model)?;
    let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
    let is_oauth_token = api_key.contains("sk-ant-oat");
    let is_copilot = request.model.provider == "github-copilot";
    let body = AnthropicMessagesRequest {
        model: request.model.id.clone(),
        max_tokens: 4096,
        messages: anthropic_messages(&request.messages),
        stream: true,
        system: anthropic_system_prompt(&request.messages, is_oauth_token && !is_copilot),
        tools: anthropic_tools(&request.tools),
    };

    let mut request_builder = crate::shared_blocking_client()
        .post(url)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header("anthropic-version", "2023-06-01")
        .json(&body);

    if is_copilot {
        request_builder = request_builder
            .bearer_auth(api_key)
            .header("anthropic-dangerous-direct-browser-access", "true");
    } else if is_oauth_token {
        request_builder = request_builder
            .bearer_auth(api_key)
            .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
            .header(reqwest::header::USER_AGENT, "claude-cli/2.1.75")
            .header("x-app", "cli");
    } else {
        request_builder = request_builder.header("x-api-key", api_key);
    }
    request_builder = apply_model_headers(request_builder, &request.model, &request.messages);

    if should_cancel() {
        return Err(ProviderError::new("cancelled"));
    }
    let response = request_builder
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
    let mut tool_calls = ToolCallAccumulator::default();
    let mut lines = BufReader::new(response).lines();
    emit(ProviderEvent::Start);

    while let Some(line) = {
        if should_cancel() {
            return Err(ProviderError::new("cancelled"));
        }
        lines.next()
    } {
        let line =
            line.map_err(|error| ProviderError::new(format!("stream read failed: {error}")))?;
        if let Some(delta) = parse_anthropic_tool_stream_line(&line)? {
            if let Some(call) = tool_calls.push_anthropic_delta(delta)? {
                emit(ProviderEvent::ToolCall(call));
            }
        }
        for delta in parse_anthropic_stream_line(&line)? {
            if should_cancel() {
                return Err(ProviderError::new("cancelled"));
            }
            match delta {
                StreamDelta::Text(delta) => {
                    content.push_str(&delta);
                    emit(ProviderEvent::TextDelta(delta));
                }
                StreamDelta::Reasoning(delta) => emit(ProviderEvent::ReasoningDelta(delta)),
                StreamDelta::Usage(delta_usage) => merge_usage(&mut usage, delta_usage),
                StreamDelta::ToolCall(call) => emit(ProviderEvent::ToolCall(call)),
                StreamDelta::OpenAiToolCallDelta(delta) => tool_calls.push_openai_delta(delta),
            }
        }
    }

    for call in tool_calls.finish()? {
        emit(ProviderEvent::ToolCall(call));
    }
    emit_usage_if_present(&usage, emit);
    emit(ProviderEvent::Done(Box::new(AssistantMessage {
        model,
        content,
    })));
    Ok(())
}

fn anthropic_tools(definitions: &[ToolDefinition]) -> Vec<AnthropicTool> {
    definitions
        .iter()
        .map(|definition| AnthropicTool {
            name: definition.name.clone(),
            description: definition.description.clone(),
            input_schema: definition.parameters.clone(),
        })
        .collect()
}

fn anthropic_system_prompt(messages: &[ChatMessage], is_oauth_token: bool) -> Option<String> {
    let mut parts = Vec::new();
    if is_oauth_token {
        parts.push("You are Claude Code, Anthropic's official CLI for Claude.".to_string());
    }
    parts.extend(
        messages
            .iter()
            .filter(|message| message.role == MessageRole::System)
            .map(|message| message.content.clone()),
    );
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn parse_anthropic_stream_line(line: &str) -> Result<Vec<StreamDelta>, ProviderError> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') || line.starts_with("event:") {
        return Ok(Vec::new());
    }

    let Some(data) = line.strip_prefix("data:") else {
        return Ok(Vec::new());
    };
    let data = data.trim();
    if data == "[DONE]" {
        return Ok(Vec::new());
    }

    let value: serde_json::Value = serde_json::from_str(data)
        .map_err(|error| ProviderError::new(format!("invalid stream chunk: {error}")))?;
    let Some(event_type) = value.get("type").and_then(serde_json::Value::as_str) else {
        return Ok(anthropic_compat_stream_deltas(&value));
    };

    if event_type == "error" {
        return Err(ProviderError::new(data.trim().to_string()));
    }

    if event_type == "message_start" || event_type == "message_delta" {
        return Ok(anthropic_usage(value.get("message").unwrap_or(&value))
            .or_else(|| anthropic_usage(&value))
            .map(|usage| vec![StreamDelta::Usage(usage)])
            .unwrap_or_default());
    }

    if event_type != "content_block_delta" {
        return Ok(Vec::new());
    }

    Ok(value
        .get("delta")
        .and_then(anthropic_delta)
        .into_iter()
        .collect())
}

fn parse_anthropic_tool_stream_line(
    line: &str,
) -> Result<Option<AnthropicToolStreamDelta>, ProviderError> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') || line.starts_with("event:") {
        return Ok(None);
    }

    let Some(data) = line.strip_prefix("data:") else {
        return Ok(None);
    };
    let data = data.trim();
    if data == "[DONE]" {
        return Ok(None);
    }

    let value: serde_json::Value = serde_json::from_str(data)
        .map_err(|error| ProviderError::new(format!("invalid stream chunk: {error}")))?;
    let Some(event_type) = value.get("type").and_then(serde_json::Value::as_str) else {
        return Ok(None);
    };

    match event_type {
        "content_block_start" => {
            let Some(block) = value.get("content_block") else {
                return Ok(None);
            };
            if block.get("type").and_then(serde_json::Value::as_str) != Some("tool_use") {
                return Ok(None);
            }
            let index = json_usize(&value, "index").unwrap_or(0);
            let Some(id) = block.get("id").and_then(serde_json::Value::as_str) else {
                return Err(ProviderError::new("tool call missing id"));
            };
            let Some(name) = block.get("name").and_then(serde_json::Value::as_str) else {
                return Err(ProviderError::new("tool call missing name"));
            };
            let arguments = block
                .get("input")
                .filter(|input| !is_empty_json_object(input))
                .map(serde_json::Value::to_string)
                .unwrap_or_default();
            Ok(Some(AnthropicToolStreamDelta::Start {
                index,
                id: id.to_string(),
                name: name.to_string(),
                arguments,
            }))
        }
        "content_block_delta" => {
            let Some(delta) = value.get("delta") else {
                return Ok(None);
            };
            if delta.get("type").and_then(serde_json::Value::as_str) != Some("input_json_delta") {
                return Ok(None);
            }
            let index = json_usize(&value, "index").unwrap_or(0);
            let arguments = delta
                .get("partial_json")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            Ok(Some(AnthropicToolStreamDelta::Arguments {
                index,
                arguments,
            }))
        }
        "content_block_stop" => {
            let index = json_usize(&value, "index").unwrap_or(0);
            Ok(Some(AnthropicToolStreamDelta::Stop { index }))
        }
        _ => Ok(None),
    }
}

fn anthropic_usage(value: &serde_json::Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    Some(TokenUsage {
        input: usage
            .get("input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        output: usage
            .get("output_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        cache_read: usage
            .get("cache_read_input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        cache_write: usage
            .get("cache_creation_input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
    })
}

fn anthropic_compat_stream_deltas(value: &serde_json::Value) -> Vec<StreamDelta> {
    if let Some(delta) = value.get("delta").and_then(anthropic_delta) {
        return vec![delta];
    }

    value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.get("content").and_then(serde_json::Value::as_str))
        .map(|text| vec![StreamDelta::Text(text.to_string())])
        .unwrap_or_default()
}

fn anthropic_delta(delta: &serde_json::Value) -> Option<StreamDelta> {
    if let Some(text) = delta.get("text").and_then(serde_json::Value::as_str) {
        return Some(StreamDelta::Text(text.to_string()));
    }

    delta
        .get("thinking")
        .and_then(serde_json::Value::as_str)
        .map(|thinking| StreamDelta::Reasoning(thinking.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Model, ToolCall};

    #[test]
    fn parses_anthropic_stream_text_delta() {
        let deltas = parse_anthropic_stream_line(
            r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}"#,
        )
        .unwrap();

        assert_eq!(deltas, vec![StreamDelta::Text("hello".to_string())]);
    }

    #[test]
    fn parses_anthropic_stream_usage() {
        let deltas = parse_anthropic_stream_line(
            r#"data: {"type":"message_delta","usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":4,"cache_creation_input_tokens":1}}"#,
        )
        .unwrap();

        assert_eq!(
            deltas,
            vec![StreamDelta::Usage(TokenUsage {
                input: 10,
                output: 2,
                cache_read: 4,
                cache_write: 1,
            })]
        );
    }

    #[test]
    fn ignores_anthropic_stream_metadata_without_type() {
        let deltas = parse_anthropic_stream_line(
            r#"data: {"usage":{"input_tokens":10,"output_tokens":2},"model":"claude-haiku-4.5"}"#,
        )
        .unwrap();

        assert!(deltas.is_empty());
    }

    #[test]
    fn ignores_anthropic_stream_done_marker() {
        assert!(parse_anthropic_stream_line("data: [DONE]")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn parses_anthropic_stream_tool_call() {
        let mut tool_calls = ToolCallAccumulator::default();
        assert!(tool_calls
            .push_anthropic_delta(
                parse_anthropic_tool_stream_line(
                    r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"bash","input":{}}}"#,
                )
                .unwrap()
                .unwrap(),
            )
            .unwrap()
            .is_none());
        assert!(tool_calls
            .push_anthropic_delta(
                parse_anthropic_tool_stream_line(
                    r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"cargo check\"}"}}"#,
                )
                .unwrap()
                .unwrap(),
            )
            .unwrap()
            .is_none());
        let call = tool_calls
            .push_anthropic_delta(
                parse_anthropic_tool_stream_line(
                    r#"data: {"type":"content_block_stop","index":1}"#,
                )
                .unwrap()
                .unwrap(),
            )
            .unwrap()
            .unwrap();

        assert_eq!(
            call,
            ToolCall::new("toolu_1", "bash").with_argument("command", "cargo check")
        );
    }

    #[test]
    fn anthropic_provider_reports_missing_api_key() {
        let provider = AnthropicMessagesProvider;
        let events = provider.stream(ProviderRequest {
            model: Model::new("anthropic", "claude-test", "anthropic-messages")
                .with_api_key_env("EXGENT_TEST_MISSING_ANTHROPIC_API_KEY"),
            messages: vec![ChatMessage::user("hello")],
            tools: Vec::new(),
        });

        assert!(matches!(
            events.as_slice(),
            [ProviderEvent::Error(message)]
                if message.contains("EXGENT_TEST_MISSING_ANTHROPIC_API_KEY")
        ));
    }
}
