use std::io::{BufRead, BufReader};

use serde::{Deserialize, Serialize};

use super::stream::{
    emit_usage_if_present, merge_usage, resolve_api_key, ProviderError, StreamDelta,
};
use crate::{
    AssistantMessage, ChatMessage, MessageRole, ProviderAdapter, ProviderEvent, ProviderRequest,
    TokenUsage, ToolArguments, ToolCall, ToolDefinition,
};

#[derive(Clone, Debug, Default)]
pub struct GoogleGenerativeAiProvider;

impl ProviderAdapter for GoogleGenerativeAiProvider {
    fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        self.stream_events_cancellable(request, &|| false, emit);
    }

    fn stream_events_cancellable(
        &self,
        request: ProviderRequest,
        should_cancel: &dyn Fn() -> bool,
        emit: &mut dyn FnMut(ProviderEvent),
    ) {
        if let Err(error) = google_generate_content_stream(request, should_cancel, emit) {
            emit(ProviderEvent::Error(error.to_string()));
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GoogleGenerateContentRequest {
    contents: Vec<GoogleContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GoogleSystemInstruction>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<GoogleTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_config: Option<GoogleToolConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct GoogleContent {
    role: String,
    parts: Vec<GooglePart>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct GoogleSystemInstruction {
    parts: Vec<GooglePart>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct GooglePart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inline_data: Option<GoogleInlineData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_call: Option<GoogleFunctionCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_response: Option<GoogleFunctionResponse>,
    #[serde(default, skip_serializing_if = "is_false")]
    thought: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct GoogleInlineData {
    mime_type: String,
    data: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct GoogleFunctionCall {
    name: String,
    #[serde(default, skip_serializing_if = "is_empty_tool_arguments")]
    args: ToolArguments,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct GoogleFunctionResponse {
    name: String,
    response: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GoogleTool {
    function_declarations: Vec<GoogleFunctionDeclaration>,
}

#[derive(Debug, Serialize)]
struct GoogleFunctionDeclaration {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GoogleToolConfig {
    function_calling_config: GoogleFunctionCallingConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GoogleFunctionCallingConfig {
    mode: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleStreamChunk {
    #[serde(default)]
    candidates: Vec<GoogleCandidate>,
    usage_metadata: Option<GoogleUsage>,
    error: Option<GoogleError>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleCandidate {
    content: Option<GoogleContent>,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleError {
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleUsage {
    prompt_token_count: Option<u64>,
    candidates_token_count: Option<u64>,
    thoughts_token_count: Option<u64>,
    cached_content_token_count: Option<u64>,
    total_token_count: Option<u64>,
}

impl From<GoogleUsage> for TokenUsage {
    fn from(usage: GoogleUsage) -> Self {
        let cache_read = usage.cached_content_token_count.unwrap_or(0);
        let input = usage
            .prompt_token_count
            .unwrap_or(0)
            .saturating_sub(cache_read);
        let output = usage
            .candidates_token_count
            .unwrap_or(0)
            .saturating_add(usage.thoughts_token_count.unwrap_or(0));
        Self {
            input,
            output: if output == 0 {
                usage
                    .total_token_count
                    .unwrap_or(input)
                    .saturating_sub(input + cache_read)
            } else {
                output
            },
            cache_read,
            cache_write: 0,
        }
    }
}

fn google_generate_content_stream(
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
        .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta".to_string());
    let api_key = resolve_api_key(&request.model)?;
    let url = format!(
        "{}/models/{}:streamGenerateContent?alt=sse",
        base_url.trim_end_matches('/'),
        request.model.id
    );
    let tools = google_tools(&request.tools);
    let tool_config = (!tools.is_empty()).then_some(GoogleToolConfig {
        function_calling_config: GoogleFunctionCallingConfig { mode: "AUTO" },
    });
    let (system_instruction, contents) = google_messages(&request.messages);
    let body = GoogleGenerateContentRequest {
        contents,
        system_instruction,
        tools,
        tool_config,
    };

    let mut request_builder = crate::shared_blocking_client()
        .post(url)
        .header("x-goog-api-key", api_key)
        .header(reqwest::header::ACCEPT, "text/event-stream");
    for (key, value) in &request.model.headers {
        request_builder = request_builder.header(key.as_str(), value.as_str());
    }

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
    let mut lines = BufReader::new(response).lines();
    let mut content = String::new();
    let mut usage = TokenUsage::default();
    emit(ProviderEvent::Start);

    while let Some(line) = {
        if should_cancel() {
            return Err(ProviderError::new("cancelled"));
        }
        lines.next()
    } {
        let line =
            line.map_err(|error| ProviderError::new(format!("stream read failed: {error}")))?;
        for delta in parse_google_stream_line(&line)? {
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
                StreamDelta::OpenAiToolCallDelta(_) => {}
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

fn google_messages(
    messages: &[ChatMessage],
) -> (Option<GoogleSystemInstruction>, Vec<GoogleContent>) {
    let mut system_messages = Vec::new();
    let mut contents = Vec::new();

    for message in messages {
        match message.role {
            MessageRole::System => {
                if !message.content.trim().is_empty() {
                    system_messages.push(message.content.clone());
                }
            }
            MessageRole::User => {
                push_google_content(&mut contents, "user", google_user_parts(message))
            }
            MessageRole::Assistant => {
                let mut parts = Vec::new();
                if !message.content.trim().is_empty() {
                    parts.push(GooglePart::text(message.content.clone()));
                }
                parts.extend(message.tool_calls.iter().map(google_function_call_part));
                if !parts.is_empty() {
                    push_google_content(&mut contents, "model", parts);
                }
            }
            MessageRole::Tool => {
                let part = tool_result_view(message)
                    .map(google_function_response_part)
                    .unwrap_or_else(|| {
                        GooglePart::text(format!("Tool result:\n{}", message.content))
                    });
                push_google_content(&mut contents, "user", vec![part]);
            }
        }
    }

    let system_instruction = (!system_messages.is_empty()).then(|| GoogleSystemInstruction {
        parts: vec![GooglePart::text(system_messages.join("\n\n"))],
    });
    (system_instruction, contents)
}

fn push_google_content(contents: &mut Vec<GoogleContent>, role: &str, parts: Vec<GooglePart>) {
    if parts.is_empty() {
        return;
    }
    if let Some(last) = contents.last_mut().filter(|last| last.role == role) {
        last.parts.extend(parts);
        return;
    }
    contents.push(GoogleContent {
        role: role.to_string(),
        parts,
    });
}

fn google_tools(definitions: &[ToolDefinition]) -> Vec<GoogleTool> {
    if definitions.is_empty() {
        return Vec::new();
    }
    vec![GoogleTool {
        function_declarations: definitions
            .iter()
            .map(|definition| GoogleFunctionDeclaration {
                name: definition.name.clone(),
                description: definition.description.clone(),
                parameters: definition.parameters.clone(),
            })
            .collect(),
    }]
}

fn google_function_call_part(call: &ToolCall) -> GooglePart {
    GooglePart {
        text: None,
        inline_data: None,
        function_call: Some(GoogleFunctionCall {
            name: call.name.clone(),
            args: call.arguments.clone(),
        }),
        function_response: None,
        thought: false,
    }
}

fn google_function_response_part(result: ToolResultView) -> GooglePart {
    GooglePart {
        text: None,
        inline_data: None,
        function_call: None,
        function_response: Some(GoogleFunctionResponse {
            name: result.tool_name,
            response: if result.is_error {
                serde_json::json!({ "error": result.content })
            } else {
                serde_json::json!({ "output": result.content })
            },
        }),
        thought: false,
    }
}

impl GooglePart {
    fn text(text: String) -> Self {
        Self {
            text: Some(text),
            inline_data: None,
            function_call: None,
            function_response: None,
            thought: false,
        }
    }

    fn image(image: &crate::ImageContent) -> Self {
        Self {
            text: None,
            inline_data: Some(GoogleInlineData {
                mime_type: image.mime_type.clone(),
                data: image.data.clone(),
            }),
            function_call: None,
            function_response: None,
            thought: false,
        }
    }
}

fn google_user_parts(message: &ChatMessage) -> Vec<GooglePart> {
    let mut parts = Vec::new();
    if !message.content.trim().is_empty() {
        parts.push(GooglePart::text(message.content.clone()));
    }
    parts.extend(message.images.iter().map(GooglePart::image));
    parts
}

struct ToolResultView {
    tool_name: String,
    content: String,
    is_error: bool,
}

fn tool_result_view(message: &ChatMessage) -> Option<ToolResultView> {
    message.tool_name.as_ref().map(|tool_name| ToolResultView {
        tool_name: tool_name.clone(),
        content: message.content.clone(),
        is_error: message.tool_is_error.unwrap_or(false),
    })
}

fn parse_google_stream_line(line: &str) -> Result<Vec<StreamDelta>, ProviderError> {
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

    let chunk: GoogleStreamChunk = serde_json::from_str(data)
        .map_err(|error| ProviderError::new(format!("invalid stream chunk: {error}")))?;
    if let Some(error) = chunk.error {
        return Err(ProviderError::new(error.message));
    }

    let mut deltas = Vec::new();
    for candidate in chunk.candidates {
        let _finish_reason = candidate.finish_reason;
        if let Some(content) = candidate.content {
            for (index, part) in content.parts.into_iter().enumerate() {
                if let Some(text) = part.text {
                    if part.thought {
                        deltas.push(StreamDelta::Reasoning(text));
                    } else {
                        deltas.push(StreamDelta::Text(text));
                    }
                }
                if let Some(function_call) = part.function_call {
                    deltas.push(StreamDelta::ToolCall(ToolCall {
                        id: format!("{}_{}", sanitize_tool_call_name(&function_call.name), index),
                        name: function_call.name,
                        arguments: function_call.args,
                    }));
                }
            }
        }
    }
    if let Some(usage) = chunk.usage_metadata {
        deltas.push(StreamDelta::Usage(usage.into()));
    }
    Ok(deltas)
}

fn sanitize_tool_call_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "tool_call".to_string()
    } else {
        sanitized
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_empty_tool_arguments(arguments: &ToolArguments) -> bool {
    arguments.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatMessage, Model, ProviderEvent, ProviderRequest, ToolCall};

    #[test]
    fn google_provider_reports_missing_api_key() {
        let provider = GoogleGenerativeAiProvider;
        let events = provider.stream(ProviderRequest {
            model: Model::new("google", "gemini-test", "google-generative-ai")
                .with_api_key_env("EXGENT_TEST_MISSING_GEMINI_API_KEY"),
            messages: vec![ChatMessage::user("hello")],
            tools: Vec::new(),
        });

        assert!(
            matches!(events.first(), Some(ProviderEvent::Error(message)) if message.contains("EXGENT_TEST_MISSING_GEMINI_API_KEY"))
        );
    }

    #[test]
    fn converts_google_messages_with_tool_history() {
        let messages = vec![
            ChatMessage::system("You are concise."),
            ChatMessage::user("inspect"),
            ChatMessage::assistant_tool_call(
                ToolCall::new("call_1", "read").with_argument("path", "Cargo.toml"),
            ),
            ChatMessage::tool_result("call_1", "read", "workspace manifest", false),
        ];

        let (system_instruction, contents) = google_messages(&messages);

        assert_eq!(
            serde_json::to_value(system_instruction).unwrap(),
            serde_json::json!({"parts":[{"text":"You are concise."}]})
        );
        assert_eq!(
            serde_json::to_value(contents).unwrap(),
            serde_json::json!([
                {"role":"user","parts":[{"text":"inspect"}]},
                {"role":"model","parts":[{"functionCall":{"name":"read","args":{"path":"Cargo.toml"}}}]},
                {"role":"user","parts":[{"functionResponse":{"name":"read","response":{"output":"workspace manifest"}}}]}
            ])
        );
    }

    #[test]
    fn parses_google_stream_text_tool_and_usage() {
        let deltas = parse_google_stream_line(
            r#"data: {"candidates":[{"content":{"role":"model","parts":[{"text":"thinking","thought":true},{"text":"hi"},{"functionCall":{"name":"read","args":{"path":"Cargo.toml"}}}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":12,"cachedContentTokenCount":2,"candidatesTokenCount":3,"thoughtsTokenCount":4,"totalTokenCount":21}}"#,
        )
        .unwrap();

        assert_eq!(deltas[0], StreamDelta::Reasoning("thinking".to_string()));
        assert_eq!(deltas[1], StreamDelta::Text("hi".to_string()));
        assert!(
            matches!(&deltas[2], StreamDelta::ToolCall(call) if call.name == "read" && call.arguments["path"] == serde_json::json!("Cargo.toml"))
        );
        assert_eq!(
            deltas[3],
            StreamDelta::Usage(TokenUsage {
                input: 10,
                output: 7,
                cache_read: 2,
                cache_write: 0,
            })
        );
    }
}
