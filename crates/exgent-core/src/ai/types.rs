use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq)]
pub struct Model {
    pub provider: String,
    pub id: String,
    pub name: Option<String>,
    pub adapter: String,
    pub base_url: Option<String>,
    pub api_key_env: Option<String>,
    pub api_key: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub compat: Option<serde_json::Value>,
    pub reasoning: Option<bool>,
    pub thinking_level_map: BTreeMap<String, Option<String>>,
    pub input: Vec<String>,
    pub output: Vec<String>,
    pub cost: Option<ModelCost>,
    pub context_window: Option<u64>,
    pub max_tokens: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

impl fmt::Debug for Model {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Model")
            .field("provider", &self.provider)
            .field("id", &self.id)
            .field("name", &self.name)
            .field("adapter", &self.adapter)
            .field("base_url", &self.base_url)
            .field("api_key_env", &self.api_key_env)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("reasoning", &self.reasoning)
            .field("input", &self.input)
            .field("output", &self.output)
            .field("context_window", &self.context_window)
            .field("max_tokens", &self.max_tokens)
            .finish()
    }
}

impl Model {
    pub fn new(
        provider: impl Into<String>,
        id: impl Into<String>,
        adapter: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            id: id.into(),
            name: None,
            adapter: adapter.into(),
            base_url: None,
            api_key_env: None,
            api_key: None,
            headers: BTreeMap::new(),
            compat: None,
            reasoning: None,
            thinking_level_map: BTreeMap::new(),
            input: Vec::new(),
            output: Vec::new(),
            cost: None,
            context_window: None,
            max_tokens: None,
        }
    }

    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    pub fn with_api_key_env(mut self, api_key_env: impl Into<String>) -> Self {
        self.api_key_env = Some(api_key_env.into());
        self
    }

    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImageContent {
    pub data: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

impl ImageContent {
    pub fn new(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Self {
            data: data.into(),
            mime_type: mime_type.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub reasoning: Option<String>,
    pub images: Vec<ImageContent>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_is_error: Option<bool>,
}

impl ChatMessage {
    fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            reasoning: None,
            images: Vec::new(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
            tool_is_error: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(MessageRole::User, content)
    }

    pub fn user_with_images(content: impl Into<String>, images: Vec<ImageContent>) -> Self {
        let mut message = Self::user(content);
        message.images = images;
        message
    }

    pub fn with_images(mut self, images: Vec<ImageContent>) -> Self {
        self.images = images;
        self
    }

    pub fn with_reasoning(mut self, reasoning: impl Into<String>) -> Self {
        self.reasoning = Some(reasoning.into());
        self
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Assistant, content)
    }

    pub fn assistant_with_reasoning(
        content: impl Into<String>,
        reasoning: impl Into<String>,
    ) -> Self {
        Self::assistant(content).with_reasoning(reasoning)
    }

    pub fn assistant_tool_call(call: ToolCall) -> Self {
        Self::assistant_tool_calls("", vec![call])
    }

    pub fn assistant_tool_calls(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        let mut message = Self::new(MessageRole::Assistant, content);
        message.tool_calls = tool_calls;
        message
    }

    pub fn tool(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Tool, content)
    }

    pub fn tool_result(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        let mut message = Self::new(MessageRole::Tool, content);
        message.tool_call_id = Some(tool_call_id.into());
        message.tool_name = Some(tool_name.into());
        message.tool_is_error = Some(is_error);
        message
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(MessageRole::System, content)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderRequest {
    pub model: Model,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssistantMessage {
    pub model: Model,
    pub content: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub label: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub execution_mode: ToolExecutionMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_snippet: Option<String>,
}

impl ToolDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        let name = name.into();
        Self {
            label: name.clone(),
            name,
            description: description.into(),
            parameters,
            execution_mode: ToolExecutionMode::Sequential,
            prompt_snippet: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn with_execution_mode(mut self, execution_mode: ToolExecutionMode) -> Self {
        self.execution_mode = execution_mode;
        self
    }

    pub fn with_prompt_snippet(mut self, prompt_snippet: impl Into<String>) -> Self {
        self.prompt_snippet = Some(prompt_snippet.into());
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionMode {
    #[default]
    Sequential,
    Parallel,
}

pub type ToolArguments = BTreeMap<String, serde_json::Value>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: ToolArguments,
}

impl ToolCall {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments: BTreeMap::new(),
        }
    }

    pub fn with_argument(
        mut self,
        key: impl Into<String>,
        value: impl Into<serde_json::Value>,
    ) -> Self {
        self.arguments.insert(key.into(), value.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProviderEvent {
    Start,
    TextDelta(String),
    ReasoningDelta(String),
    Usage(TokenUsage),
    ToolCall(ToolCall),
    Done(Box<AssistantMessage>),
    Error(String),
}

pub trait ProviderAdapter {
    fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent));

    fn stream_events_cancellable(
        &self,
        request: ProviderRequest,
        should_cancel: &dyn Fn() -> bool,
        emit: &mut dyn FnMut(ProviderEvent),
    ) {
        let _ = should_cancel;
        self.stream_events(request, emit);
    }

    #[allow(dead_code)]
    fn stream(&self, request: ProviderRequest) -> Vec<ProviderEvent> {
        let mut events = Vec::new();
        self.stream_events_cancellable(request, &|| false, &mut |event| events.push(event));
        events
    }
}
