use std::io::{BufRead, BufReader};

use serde::{Deserialize, Serialize};

use super::stream::{
    apply_model_headers, emit_usage_if_present, merge_usage, parse_tool_arguments, resolve_api_key,
    ProviderError, StreamDelta, ToolCallAccumulator,
};
use crate::{
    llm_convert::openai_responses_input, AssistantMessage, ProviderAdapter, ProviderEvent,
    ProviderRequest, TokenUsage, ToolCall, ToolDefinition,
};

#[derive(Clone, Debug, Default)]
pub struct OpenAiResponsesProvider;

impl ProviderAdapter for OpenAiResponsesProvider {
    fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        if let Err(error) = openai_responses_stream(request, emit) {
            emit(ProviderEvent::Error(error.to_string()));
        }
    }
}

#[derive(Debug, Serialize)]
struct OpenAiResponsesRequest {
    model: String,
    input: Vec<serde_json::Value>,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OpenAiResponsesTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub(crate) struct OpenAiResponsesTool {
    #[serde(rename = "type")]
    tool_type: &'static str,
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponsesStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    delta: Option<String>,
    text: Option<String>,
    response: Option<OpenAiResponsesResponse>,
    item: Option<OpenAiResponsesOutputItem>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponsesOutputItem {
    id: Option<String>,
    #[serde(rename = "type")]
    item_type: String,
    call_id: Option<String>,
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponsesResponse {
    usage: Option<OpenAiResponsesUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponsesUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    input_tokens_details: Option<OpenAiResponsesInputTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponsesInputTokensDetails {
    cached_tokens: Option<u64>,
}

impl From<OpenAiResponsesUsage> for TokenUsage {
    fn from(usage: OpenAiResponsesUsage) -> Self {
        let input = usage.input_tokens.unwrap_or(0);
        let output = usage
            .output_tokens
            .unwrap_or_else(|| usage.total_tokens.unwrap_or(input).saturating_sub(input));
        Self {
            input,
            output,
            cache_read: usage
                .input_tokens_details
                .and_then(|details| details.cached_tokens)
                .unwrap_or(0),
            cache_write: 0,
        }
    }
}

fn openai_responses_stream(
    request: ProviderRequest,
    emit: &mut dyn FnMut(ProviderEvent),
) -> Result<(), ProviderError> {
    let base_url = request
        .model
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let api_key = resolve_api_key(&request.model)?;
    let url = format!("{}/responses", base_url.trim_end_matches('/'));
    let tools = openai_responses_tools(&request.tools);
    let tool_choice = (!tools.is_empty()).then_some("auto");
    let body = OpenAiResponsesRequest {
        model: request.model.id.clone(),
        input: openai_responses_input(&request.messages),
        stream: true,
        tools,
        tool_choice,
    };

    let request_builder = reqwest::blocking::Client::new()
        .post(url)
        .bearer_auth(api_key)
        .header(reqwest::header::ACCEPT, "text/event-stream");
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
        for delta in parse_openai_responses_stream_line(&line)? {
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

pub(crate) fn openai_responses_tools(definitions: &[ToolDefinition]) -> Vec<OpenAiResponsesTool> {
    definitions
        .iter()
        .map(|definition| OpenAiResponsesTool {
            tool_type: "function",
            name: definition.name.clone(),
            description: definition.description.clone(),
            parameters: definition.parameters.clone(),
        })
        .collect()
}

pub(crate) fn parse_openai_responses_stream_line(
    line: &str,
) -> Result<Vec<StreamDelta>, ProviderError> {
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

    let event: OpenAiResponsesStreamEvent = serde_json::from_str(data)
        .map_err(|error| ProviderError::new(format!("invalid stream chunk: {error}")))?;
    if event.event_type == "error" || event.event_type == "response.failed" {
        return Err(ProviderError::new(data.to_string()));
    }

    match event.event_type.as_str() {
        "response.output_text.delta" => {
            Ok(event.delta.into_iter().map(StreamDelta::Text).collect())
        }
        "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => Ok(event
            .delta
            .into_iter()
            .map(StreamDelta::Reasoning)
            .collect()),
        "response.output_text.done" => Ok(event.text.into_iter().map(StreamDelta::Text).collect()),
        "response.output_item.done" => event
            .item
            .and_then(openai_responses_tool_call)
            .map(|result| result.map(|call| vec![StreamDelta::ToolCall(call)]))
            .unwrap_or_else(|| Ok(Vec::new())),
        "response.completed" => Ok(event
            .response
            .and_then(|response| response.usage)
            .map(|usage| vec![StreamDelta::Usage(usage.into())])
            .unwrap_or_default()),
        _ => Ok(Vec::new()),
    }
}

fn openai_responses_tool_call(
    item: OpenAiResponsesOutputItem,
) -> Option<Result<ToolCall, ProviderError>> {
    if item.item_type != "function_call" {
        return None;
    }

    let Some(name) = item.name else {
        return Some(Err(ProviderError::new("tool call missing name")));
    };
    let id = item
        .call_id
        .or(item.id)
        .unwrap_or_else(|| format!("tool_call_{name}"));
    let arguments = item.arguments.unwrap_or_default();

    Some(parse_tool_arguments(&arguments).map(|arguments| ToolCall {
        id,
        name,
        arguments,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_responses_stream_delta() {
        let deltas = parse_openai_responses_stream_line(
            r#"data: {"type":"response.output_text.delta","delta":"hello"}"#,
        )
        .unwrap();

        assert_eq!(deltas, vec![StreamDelta::Text("hello".to_string())]);
    }

    #[test]
    fn parses_openai_responses_stream_tool_call() {
        let deltas = parse_openai_responses_stream_line(
            r#"data: {"type":"response.output_item.done","item":{"type":"function_call","call_id":"call_1","name":"read","arguments":"{\"path\":\"Cargo.toml\",\"offset\":2}"}}"#,
        )
        .unwrap();

        assert_eq!(
            deltas,
            vec![StreamDelta::ToolCall(
                ToolCall::new("call_1", "read")
                    .with_argument("path", "Cargo.toml")
                    .with_argument("offset", 2)
            )]
        );
    }

    #[test]
    fn parses_openai_responses_stream_usage() {
        let deltas = parse_openai_responses_stream_line(
            r#"data: {"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":2,"input_tokens_details":{"cached_tokens":4}}}}"#,
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
}
