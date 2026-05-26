use std::io::{BufRead, BufReader};

use serde::{Deserialize, Serialize};

use super::stream::{
    apply_model_headers, emit_usage_if_present, merge_usage, resolve_api_key, OpenAiToolCallDelta,
    ProviderError, StreamDelta, ToolCallAccumulator,
};
use crate::{
    llm_convert::{openai_chat_messages, OpenAiChatMessage},
    AssistantMessage, ProviderAdapter, ProviderEvent, ProviderRequest, TokenUsage, ToolDefinition,
};

#[derive(Clone, Debug, Default)]
pub struct OpenAiCompatibleProvider;

impl ProviderAdapter for OpenAiCompatibleProvider {
    fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        self.stream_events_cancellable(request, &|| false, emit);
    }

    fn stream_events_cancellable(
        &self,
        request: ProviderRequest,
        should_cancel: &dyn Fn() -> bool,
        emit: &mut dyn FnMut(ProviderEvent),
    ) {
        if let Err(error) = openai_chat_completion_stream(request, should_cancel, emit) {
            emit(ProviderEvent::Error(error.to_string()));
        }
    }
}

#[derive(Debug, Serialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiChatMessage>,
    stream: bool,
    stream_options: OpenAiStreamOptions,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OpenAiTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct OpenAiStreamOptions {
    include_usage: bool,
}

#[derive(Debug, Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: &'static str,
    function: OpenAiToolFunction,
}

#[derive(Debug, Serialize)]
struct OpenAiToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChunk {
    #[serde(default)]
    choices: Vec<OpenAiStreamChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiStreamDelta,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamDelta {
    content: Option<String>,
    reasoning_content: Option<String>,
    reasoning: Option<String>,
    reasoning_text: Option<String>,
    tool_calls: Option<Vec<OpenAiToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
    prompt_tokens_details: Option<OpenAiPromptTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct OpenAiPromptTokensDetails {
    cached_tokens: Option<u64>,
}

impl OpenAiStreamDelta {
    fn into_stream_deltas(self) -> Vec<StreamDelta> {
        let mut deltas = Vec::new();
        if let Some(content) = self.content {
            deltas.push(StreamDelta::Text(content));
        }

        if let Some(reasoning) = self
            .reasoning_content
            .or(self.reasoning)
            .or(self.reasoning_text)
        {
            deltas.push(StreamDelta::Reasoning(reasoning));
        }

        deltas.extend(
            self.tool_calls
                .unwrap_or_default()
                .into_iter()
                .map(StreamDelta::OpenAiToolCallDelta),
        );
        deltas
    }
}

impl From<OpenAiUsage> for TokenUsage {
    fn from(usage: OpenAiUsage) -> Self {
        let input = usage.prompt_tokens.unwrap_or(0);
        let output = usage
            .completion_tokens
            .unwrap_or_else(|| usage.total_tokens.unwrap_or(input).saturating_sub(input));
        Self {
            input,
            output,
            cache_read: usage
                .prompt_tokens_details
                .and_then(|details| details.cached_tokens)
                .unwrap_or(0),
            cache_write: 0,
        }
    }
}

fn openai_chat_completion_stream(
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
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let api_key = resolve_api_key(&request.model)?;
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let tools = openai_tools(&request.tools);
    let tool_choice = (!tools.is_empty()).then_some("auto");
    let body = OpenAiChatRequest {
        model: request.model.id.clone(),
        messages: openai_chat_messages(&request.messages, &request.model),
        stream: true,
        stream_options: OpenAiStreamOptions {
            include_usage: true,
        },
        tools,
        tool_choice,
    };

    let request_builder = crate::shared_blocking_client()
        .post(url)
        .bearer_auth(api_key)
        .header(reqwest::header::ACCEPT, "text/event-stream");
    let request_builder = apply_model_headers(request_builder, &request.model, &request.messages);

    if should_cancel() {
        return Err(ProviderError::new("cancelled"));
    }
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
        for delta in parse_openai_stream_line(&line)? {
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

fn openai_tools(definitions: &[ToolDefinition]) -> Vec<OpenAiTool> {
    definitions
        .iter()
        .map(|definition| OpenAiTool {
            tool_type: "function",
            function: OpenAiToolFunction {
                name: definition.name.clone(),
                description: definition.description.clone(),
                parameters: definition.parameters.clone(),
            },
        })
        .collect()
}

fn parse_openai_stream_line(line: &str) -> Result<Vec<StreamDelta>, ProviderError> {
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

    let chunk: OpenAiStreamChunk = serde_json::from_str(data)
        .map_err(|error| ProviderError::new(format!("invalid stream chunk: {error}")))?;
    if let Some(usage) = chunk.usage {
        return Ok(vec![StreamDelta::Usage(usage.into())]);
    }
    Ok(chunk
        .choices
        .into_iter()
        .flat_map(|choice| choice.delta.into_stream_deltas())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::super::stream::OpenAiFunctionDelta;
    use super::*;
    use crate::{ChatMessage, Model, ToolCall};

    #[test]
    fn openai_provider_reports_missing_api_key() {
        let provider = OpenAiCompatibleProvider;
        let events = provider.stream(ProviderRequest {
            model: Model::new("openai", "gpt-test", "openai-completions")
                .with_api_key_env("EXGENT_TEST_MISSING_API_KEY"),
            messages: vec![ChatMessage::user("hello")],
            tools: Vec::new(),
        });

        assert!(matches!(
            events.as_slice(),
            [ProviderEvent::Error(message)] if message.contains("EXGENT_TEST_MISSING_API_KEY")
        ));
    }

    #[test]
    fn parses_openai_stream_content_delta() {
        let deltas =
            parse_openai_stream_line(r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#)
                .unwrap();

        assert_eq!(deltas, vec![StreamDelta::Text("hello".to_string())]);
    }

    #[test]
    fn parses_openai_stream_reasoning_delta() {
        let deltas = parse_openai_stream_line(
            r#"data: {"choices":[{"delta":{"reasoning_content":"thinking"}}]}"#,
        )
        .unwrap();

        assert_eq!(deltas, vec![StreamDelta::Reasoning("thinking".to_string())]);
    }

    #[test]
    fn parses_openai_stream_usage() {
        let deltas = parse_openai_stream_line(
            r#"data: {"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":4}}}"#,
        )
        .unwrap();

        assert_eq!(
            deltas,
            vec![StreamDelta::Usage(TokenUsage {
                input: 10,
                output: 2,
                cache_read: 4,
                cache_write: 0,
            })]
        );
    }

    #[test]
    fn parses_openai_stream_tool_call_delta() {
        let deltas = parse_openai_stream_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read","arguments":"{\"path\":\"Cargo.toml\"}"}}]}}]}"#,
        )
        .unwrap();

        assert_eq!(
            deltas,
            vec![StreamDelta::OpenAiToolCallDelta(OpenAiToolCallDelta {
                index: 0,
                id: Some("call_1".to_string()),
                function: Some(OpenAiFunctionDelta {
                    name: Some("read".to_string()),
                    arguments: Some(r#"{"path":"Cargo.toml"}"#.to_string()),
                }),
            })]
        );
    }

    #[test]
    fn ignores_openai_stream_done_marker() {
        assert!(parse_openai_stream_line("data: [DONE]").unwrap().is_empty());
    }

    #[test]
    fn openai_tool_call_accumulator_finishes_delta() {
        let mut accumulator = ToolCallAccumulator::default();
        accumulator.push_openai_delta(OpenAiToolCallDelta {
            index: 0,
            id: Some("call_1".to_string()),
            function: Some(OpenAiFunctionDelta {
                name: Some("read".to_string()),
                arguments: Some(r#"{"path":"Cargo.toml"}"#.to_string()),
            }),
        });

        assert_eq!(
            accumulator.finish().unwrap(),
            vec![ToolCall::new("call_1", "read").with_argument("path", "Cargo.toml")]
        );
    }
}
