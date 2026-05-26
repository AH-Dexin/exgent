use std::{collections::BTreeMap, env, error::Error, fmt};

use serde::Deserialize;

use super::super::{ChatMessage, MessageRole, Model, ProviderEvent, TokenUsage, ToolArguments, ToolCall};

#[derive(Debug)]
pub(crate) struct ProviderError {
    message: String,
}

impl ProviderError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ProviderError {}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum StreamDelta {
    Text(String),
    Reasoning(String),
    Usage(TokenUsage),
    ToolCall(ToolCall),
    OpenAiToolCallDelta(OpenAiToolCallDelta),
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct OpenAiToolCallDelta {
    pub(crate) index: usize,
    pub(crate) id: Option<String>,
    pub(crate) function: Option<OpenAiFunctionDelta>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct OpenAiFunctionDelta {
    pub(crate) name: Option<String>,
    pub(crate) arguments: Option<String>,
}

#[derive(Default)]
pub(crate) struct ToolCallAccumulator {
    calls: BTreeMap<usize, PendingToolCall>,
}

#[derive(Default)]
struct PendingToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl ToolCallAccumulator {
    pub(crate) fn push_openai_delta(&mut self, delta: OpenAiToolCallDelta) {
        let pending = self.calls.entry(delta.index).or_default();
        if let Some(id) = delta.id {
            pending.id = Some(id);
        }
        if let Some(function) = delta.function {
            if let Some(name) = function.name {
                pending.name = Some(name);
            }
            if let Some(arguments) = function.arguments {
                pending.arguments.push_str(&arguments);
            }
        }
    }

    pub(crate) fn push_anthropic_delta(
        &mut self,
        delta: AnthropicToolStreamDelta,
    ) -> Result<Option<ToolCall>, ProviderError> {
        match delta {
            AnthropicToolStreamDelta::Start {
                index,
                id,
                name,
                arguments,
            } => {
                let pending = self.calls.entry(index).or_default();
                pending.id = Some(id);
                pending.name = Some(name);
                pending.arguments.push_str(&arguments);
                Ok(None)
            }
            AnthropicToolStreamDelta::Arguments { index, arguments } => {
                self.calls
                    .entry(index)
                    .or_default()
                    .arguments
                    .push_str(&arguments);
                Ok(None)
            }
            AnthropicToolStreamDelta::Stop { index } => self
                .calls
                .remove(&index)
                .map(|pending| pending.into_tool_call(index))
                .transpose(),
        }
    }

    pub(crate) fn finish(self) -> Result<Vec<ToolCall>, ProviderError> {
        self.calls
            .into_iter()
            .map(|(index, pending)| pending.into_tool_call(index))
            .collect()
    }
}

impl PendingToolCall {
    fn into_tool_call(self, index: usize) -> Result<ToolCall, ProviderError> {
        let name = self
            .name
            .ok_or_else(|| ProviderError::new("tool call missing name"))?;
        let id = self.id.unwrap_or_else(|| format!("tool_call_{index}"));
        Ok(ToolCall {
            id,
            name,
            arguments: parse_tool_arguments(&self.arguments)?,
        })
    }
}

pub(crate) enum AnthropicToolStreamDelta {
    Start {
        index: usize,
        id: String,
        name: String,
        arguments: String,
    },
    Arguments {
        index: usize,
        arguments: String,
    },
    Stop {
        index: usize,
    },
}

pub(crate) fn merge_usage(total: &mut TokenUsage, usage: TokenUsage) {
    total.input = total.input.max(usage.input);
    total.output = total.output.max(usage.output);
    total.cache_read = total.cache_read.max(usage.cache_read);
    total.cache_write = total.cache_write.max(usage.cache_write);
}

pub(crate) fn emit_usage_if_present(usage: &TokenUsage, emit: &mut dyn FnMut(ProviderEvent)) {
    if usage.input > 0 || usage.output > 0 || usage.cache_read > 0 || usage.cache_write > 0 {
        emit(ProviderEvent::Usage(usage.clone()));
    }
}

pub(crate) fn apply_model_headers(
    mut request_builder: reqwest::blocking::RequestBuilder,
    model: &Model,
    messages: &[ChatMessage],
) -> reqwest::blocking::RequestBuilder {
    for (key, value) in &model.headers {
        request_builder = request_builder.header(key.as_str(), value.as_str());
    }

    if model.provider != "github-copilot" {
        return request_builder;
    }

    request_builder
        .header("X-Initiator", copilot_initiator(messages))
        .header("Openai-Intent", "conversation-edits")
}

fn copilot_initiator(messages: &[ChatMessage]) -> &'static str {
    match messages.last().map(|message| &message.role) {
        Some(MessageRole::User) => "user",
        Some(_) => "agent",
        None => "user",
    }
}

pub(crate) fn resolve_api_key(model: &Model) -> Result<String, ProviderError> {
    if let Some(api_key) = model.api_key.as_deref().filter(|value| !value.is_empty()) {
        return Ok(api_key.to_string());
    }

    let env_name = model
        .api_key_env
        .as_deref()
        .unwrap_or("OPENAI_API_KEY")
        .to_string();
    env::var(&env_name)
        .map_err(|_| ProviderError::new(format!("missing API key env var: {env_name}")))
}

pub(crate) fn json_usize(value: &serde_json::Value, key: &str) -> Option<usize> {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
}

pub(crate) fn is_empty_json_object(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .map(|object| object.is_empty())
        .unwrap_or(false)
}

pub(crate) fn parse_tool_arguments(arguments: &str) -> Result<ToolArguments, ProviderError> {
    let arguments = arguments.trim();
    if arguments.is_empty() {
        return Ok(BTreeMap::new());
    }

    let value: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|error| ProviderError::new(format!("invalid tool arguments: {error}")))?;
    let Some(object) = value.as_object() else {
        return Err(ProviderError::new("tool arguments must be a JSON object"));
    };

    Ok(object
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_tool_arguments_without_losing_json_types() {
        let arguments = parse_tool_arguments(
            r#"{"path":"Cargo.toml","offset":2,"replace_all":true,"filters":{"kind":"rs"}}"#,
        )
        .unwrap();

        assert_eq!(arguments["path"], json!("Cargo.toml"));
        assert_eq!(arguments["offset"], json!(2));
        assert_eq!(arguments["replace_all"], json!(true));
        assert_eq!(arguments["filters"], json!({"kind": "rs"}));
    }
}
