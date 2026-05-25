//! Provider and adapter primitives for exgent.

use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fmt,
    io::{self, BufRead, BufReader},
    sync::Arc,
};

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
        }
    }

    pub fn tool(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Tool,
            content: content.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderRequest {
    pub model: Model,
    pub messages: Vec<ChatMessage>,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: BTreeMap<String, String>,
}

impl ToolCall {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments: BTreeMap::new(),
        }
    }

    pub fn with_argument(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
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

    fn stream(&self, request: ProviderRequest) -> Vec<ProviderEvent> {
        let mut events = Vec::new();
        self.stream_events(request, &mut |event| events.push(event));
        events
    }
}

#[derive(Clone)]
pub struct DynamicProvider {
    adapter_name: String,
    adapter: Arc<dyn ProviderAdapter + Send + Sync>,
}

impl DynamicProvider {
    pub fn new(
        adapter_name: impl Into<String>,
        adapter: impl ProviderAdapter + Send + Sync + 'static,
    ) -> Self {
        Self {
            adapter_name: adapter_name.into(),
            adapter: Arc::new(adapter),
        }
    }

    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }
}

impl fmt::Debug for DynamicProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DynamicProvider")
            .field("adapter_name", &self.adapter_name)
            .finish()
    }
}

impl ProviderAdapter for DynamicProvider {
    fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        self.adapter.stream_events(request, emit);
    }
}

#[derive(Clone)]
pub struct ProviderRegistry {
    factories: BTreeMap<String, Arc<dyn Fn() -> DynamicProvider + Send + Sync>>,
    subscription_providers: Vec<SubscriptionProvider>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionProvider {
    pub id: String,
    pub name: String,
}

impl ProviderRegistry {
    pub fn builtin() -> Self {
        let mut registry = Self::new();
        registry
            .register("fake", || DynamicProvider::new("fake", FakeProvider))
            .expect("built-in provider adapters must be unique");
        registry
            .register("openai-completions", || {
                DynamicProvider::new("openai-completions", OpenAiCompatibleProvider)
            })
            .expect("built-in provider adapters must be unique");
        registry
            .register("openai-responses", || {
                DynamicProvider::new("openai-responses", OpenAiResponsesProvider)
            })
            .expect("built-in provider adapters must be unique");
        registry
            .register("anthropic-messages", || {
                DynamicProvider::new("anthropic-messages", AnthropicMessagesProvider)
            })
            .expect("built-in provider adapters must be unique");
        registry
            .register_subscription_provider("anthropic", "Anthropic (Claude Pro/Max)")
            .expect("built-in subscription providers must be unique");
        registry
            .register_subscription_provider("github-copilot", "GitHub Copilot")
            .expect("built-in subscription providers must be unique");
        registry
            .register_subscription_provider("openai-codex", "ChatGPT Plus/Pro (Codex Subscription)")
            .expect("built-in subscription providers must be unique");
        registry
    }

    pub fn new() -> Self {
        Self {
            factories: BTreeMap::new(),
            subscription_providers: Vec::new(),
        }
    }

    pub fn register(
        &mut self,
        adapter: impl Into<String>,
        factory: impl Fn() -> DynamicProvider + Send + Sync + 'static,
    ) -> io::Result<()> {
        let adapter = adapter.into();
        if self.factories.contains_key(&adapter) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("provider adapter already registered: {adapter}"),
            ));
        }

        self.factories.insert(adapter, Arc::new(factory));
        Ok(())
    }

    pub fn register_subscription_provider(
        &mut self,
        id: impl Into<String>,
        name: impl Into<String>,
    ) -> io::Result<()> {
        let id = id.into();
        if self
            .subscription_providers
            .iter()
            .any(|provider| provider.id == id)
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("subscription provider already registered: {id}"),
            ));
        }

        self.subscription_providers.push(SubscriptionProvider {
            id,
            name: name.into(),
        });
        Ok(())
    }

    pub fn subscription_providers(&self) -> &[SubscriptionProvider] {
        &self.subscription_providers
    }

    pub fn provider_for_model(&self, model: &Model) -> DynamicProvider {
        self.factories
            .get(&model.adapter)
            .map(|factory| factory())
            .unwrap_or_else(|| {
                DynamicProvider::new(
                    model.adapter.clone(),
                    UnsupportedProvider {
                        adapter: model.adapter.clone(),
                    },
                )
            })
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

#[derive(Clone, Debug, Default)]
pub struct FakeProvider;

impl ProviderAdapter for FakeProvider {
    fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        if let Some(tool_result) = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::Tool)
        {
            let text = format!("fake tool result:\n{}", tool_result.content);
            emit(ProviderEvent::Start);
            emit(ProviderEvent::TextDelta(text.clone()));
            emit(ProviderEvent::Done(Box::new(AssistantMessage {
                model: request.model,
                content: text,
            })));
            return;
        }

        let prompt = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::User)
            .map(|message| message.content.as_str())
            .unwrap_or("");

        if let Some(tool_call) = parse_fake_tool_call(prompt) {
            emit(ProviderEvent::ToolCall(tool_call));
            return;
        }

        let text = if prompt == "history count" {
            format!("fake history count: {}", request.messages.len())
        } else if prompt == "system prompt" {
            request
                .messages
                .iter()
                .find(|message| message.role == MessageRole::System)
                .map(|message| format!("fake system prompt:\n{}", message.content))
                .unwrap_or_else(|| "fake system prompt: missing".to_string())
        } else if prompt.trim().is_empty() {
            "fake response".to_string()
        } else {
            format!("fake response: {prompt}")
        };

        emit(ProviderEvent::Start);
        emit(ProviderEvent::TextDelta(text.clone()));
        emit(ProviderEvent::Done(Box::new(AssistantMessage {
            model: request.model,
            content: text,
        })));
    }
}

pub fn fake_model() -> Model {
    Model::new("fake", "fake-chat", "fake")
}

pub fn built_in_models() -> Vec<Model> {
    generated_models()
}

pub fn generated_models() -> Vec<Model> {
    let providers: BTreeMap<String, GeneratedProviderModels> =
        serde_json::from_str(include_str!("generated_models.json"))
            .expect("generated_models.json must be valid");
    providers
        .into_iter()
        .flat_map(|(provider, provider_models)| {
            provider_models.models.into_iter().map(move |(id, model)| {
                model.into_model(provider.clone(), id, &provider_models.defaults)
            })
        })
        .collect()
}

#[derive(Debug, Default, Deserialize)]
struct GeneratedProviderModels {
    #[serde(default)]
    defaults: GeneratedModelDefaults,
    models: BTreeMap<String, GeneratedModel>,
}

#[derive(Debug, Default, Deserialize)]
struct GeneratedModelDefaults {
    base_url: Option<String>,
    api_key_env: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct GeneratedModel {
    name: Option<String>,
    adapter: String,
    base_url: Option<String>,
    api_key_env: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    compat: Option<serde_json::Value>,
    reasoning: Option<bool>,
    #[serde(default)]
    thinking_level_map: BTreeMap<String, Option<String>>,
    #[serde(default)]
    input: Vec<String>,
    #[serde(default)]
    output: Vec<String>,
    cost: Option<ModelCost>,
    context_window: Option<u64>,
    max_tokens: Option<u64>,
}

impl GeneratedModel {
    fn into_model(self, provider: String, id: String, defaults: &GeneratedModelDefaults) -> Model {
        let mut model = Model::new(provider, id, self.adapter);
        model.name = self.name;
        model.base_url = self.base_url.or_else(|| defaults.base_url.clone());
        model.api_key_env = self.api_key_env.or_else(|| defaults.api_key_env.clone());
        model.headers = if self.headers.is_empty() {
            defaults.headers.clone()
        } else {
            self.headers
        };
        model.compat = self.compat;
        model.reasoning = self.reasoning;
        model.thinking_level_map = self.thinking_level_map;
        model.input = self.input;
        model.output = self.output;
        model.cost = self.cost;
        model.context_window = self.context_window;
        model.max_tokens = self.max_tokens;
        model
    }
}

#[derive(Clone, Debug, Default)]
pub struct OpenAiCompatibleProvider;

impl ProviderAdapter for OpenAiCompatibleProvider {
    fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        if let Err(error) = openai_chat_completion_stream(request, emit) {
            emit(ProviderEvent::Error(error.to_string()));
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct OpenAiResponsesProvider;

impl ProviderAdapter for OpenAiResponsesProvider {
    fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        if let Err(error) = openai_responses_stream(request, emit) {
            emit(ProviderEvent::Error(error.to_string()));
        }
    }
}

#[derive(Clone, Debug)]
pub struct UnsupportedProvider {
    adapter: String,
}

impl ProviderAdapter for UnsupportedProvider {
    fn stream_events(&self, _request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        emit(ProviderEvent::Error(format!(
            "unsupported adapter: {}",
            self.adapter
        )));
    }
}

#[derive(Clone, Debug, Default)]
pub struct AnthropicMessagesProvider;

impl ProviderAdapter for AnthropicMessagesProvider {
    fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        if let Err(error) = anthropic_messages_stream(request, emit) {
            emit(ProviderEvent::Error(error.to_string()));
        }
    }
}

#[derive(Debug)]
pub struct ProviderError {
    message: String,
}

impl ProviderError {
    fn new(message: impl Into<String>) -> Self {
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

#[derive(Debug, Serialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiChatMessage>,
    stream: bool,
    stream_options: OpenAiStreamOptions,
}

#[derive(Debug, Serialize)]
struct OpenAiChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct OpenAiStreamOptions {
    include_usage: bool,
}

#[derive(Debug, Serialize)]
struct OpenAiResponsesRequest {
    model: String,
    input: Vec<OpenAiResponsesInputMessage>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct OpenAiResponsesInputMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct AnthropicMessagesRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
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
}

#[derive(Debug, Deserialize)]
struct OpenAiResponsesStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    delta: Option<String>,
    text: Option<String>,
    response: Option<OpenAiResponsesResponse>,
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

impl OpenAiStreamDelta {
    fn into_stream_delta(self) -> Option<StreamDelta> {
        if let Some(content) = self.content {
            return Some(StreamDelta::Text(content));
        }

        self.reasoning_content
            .or(self.reasoning)
            .or(self.reasoning_text)
            .map(StreamDelta::Reasoning)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StreamDelta {
    Text(String),
    Reasoning(String),
    Usage(TokenUsage),
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

fn openai_chat_completion_stream(
    request: ProviderRequest,
    emit: &mut dyn FnMut(ProviderEvent),
) -> Result<(), ProviderError> {
    let base_url = request
        .model
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let api_key = resolve_api_key(&request.model)?;
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = OpenAiChatRequest {
        model: request.model.id.clone(),
        messages: request
            .messages
            .iter()
            .map(|message| OpenAiChatMessage {
                role: openai_role(&message.role).to_string(),
                content: message.content.clone(),
            })
            .collect(),
        stream: true,
        stream_options: OpenAiStreamOptions {
            include_usage: true,
        },
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
        for delta in parse_openai_stream_line(&line)? {
            match delta {
                StreamDelta::Text(delta) => {
                    content.push_str(&delta);
                    emit(ProviderEvent::TextDelta(delta));
                }
                StreamDelta::Reasoning(delta) => emit(ProviderEvent::ReasoningDelta(delta)),
                StreamDelta::Usage(delta_usage) => merge_usage(&mut usage, delta_usage),
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
    let body = OpenAiResponsesRequest {
        model: request.model.id.clone(),
        input: request
            .messages
            .iter()
            .map(|message| OpenAiResponsesInputMessage {
                role: openai_responses_role(&message.role).to_string(),
                content: message.content.clone(),
            })
            .collect(),
        stream: true,
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

fn anthropic_messages_stream(
    request: ProviderRequest,
    emit: &mut dyn FnMut(ProviderEvent),
) -> Result<(), ProviderError> {
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
        messages: request
            .messages
            .iter()
            .filter_map(anthropic_message)
            .collect(),
        stream: true,
        system: anthropic_system_prompt(&request.messages, is_oauth_token && !is_copilot),
    };

    let mut request_builder = reqwest::blocking::Client::new()
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
    let reader = BufReader::new(response);
    emit(ProviderEvent::Start);

    for line in reader.lines() {
        let line =
            line.map_err(|error| ProviderError::new(format!("stream read failed: {error}")))?;
        for delta in parse_anthropic_stream_line(&line)? {
            match delta {
                StreamDelta::Text(delta) => {
                    content.push_str(&delta);
                    emit(ProviderEvent::TextDelta(delta));
                }
                StreamDelta::Reasoning(delta) => emit(ProviderEvent::ReasoningDelta(delta)),
                StreamDelta::Usage(delta_usage) => merge_usage(&mut usage, delta_usage),
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

fn merge_usage(total: &mut TokenUsage, usage: TokenUsage) {
    total.input = total.input.max(usage.input);
    total.output = total.output.max(usage.output);
    total.cache_read = total.cache_read.max(usage.cache_read);
    total.cache_write = total.cache_write.max(usage.cache_write);
}

fn emit_usage_if_present(usage: &TokenUsage, emit: &mut dyn FnMut(ProviderEvent)) {
    if usage.input > 0 || usage.output > 0 || usage.cache_read > 0 || usage.cache_write > 0 {
        emit(ProviderEvent::Usage(usage.clone()));
    }
}

fn apply_model_headers(
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

fn resolve_api_key(model: &Model) -> Result<String, ProviderError> {
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

fn openai_role(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
        MessageRole::System => "system",
    }
}

fn openai_responses_role(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "user",
    }
}

fn anthropic_message(message: &ChatMessage) -> Option<AnthropicMessage> {
    match message.role {
        MessageRole::User => Some(AnthropicMessage {
            role: "user".to_string(),
            content: message.content.clone(),
        }),
        MessageRole::Assistant => Some(AnthropicMessage {
            role: "assistant".to_string(),
            content: message.content.clone(),
        }),
        MessageRole::Tool => Some(AnthropicMessage {
            role: "user".to_string(),
            content: message.content.clone(),
        }),
        MessageRole::System => None,
    }
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
        .filter_map(|choice| choice.delta.into_stream_delta())
        .collect())
}

fn parse_openai_responses_stream_line(line: &str) -> Result<Vec<StreamDelta>, ProviderError> {
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
        "response.completed" => Ok(event
            .response
            .and_then(|response| response.usage)
            .map(|usage| vec![StreamDelta::Usage(usage.into())])
            .unwrap_or_default()),
        _ => Ok(Vec::new()),
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

fn parse_fake_tool_call(prompt: &str) -> Option<ToolCall> {
    let prompt = prompt.trim();
    if let Some(path) = prompt.strip_prefix("tool read ") {
        return Some(
            ToolCall::new("fake_tool_read_1", "read")
                .with_argument("path", path.trim())
                .with_argument("offset", "0"),
        );
    }

    if let Some(rest) = prompt.strip_prefix("tool write ") {
        let (path, content) = rest.trim().split_once(' ')?;
        return Some(
            ToolCall::new("fake_tool_write_1", "write")
                .with_argument("path", path)
                .with_argument("content", content),
        );
    }

    if let Some(rest) = prompt.strip_prefix("tool edit ") {
        let (path, rest) = rest.trim().split_once(' ')?;
        let (old_text, new_text) = rest.split_once(" => ")?;
        return Some(
            ToolCall::new("fake_tool_edit_1", "edit")
                .with_argument("path", path)
                .with_argument("old_text", old_text)
                .with_argument("new_text", new_text),
        );
    }

    if let Some(command) = prompt.strip_prefix("tool bash ") {
        return Some(
            ToolCall::new("fake_tool_bash_1", "bash").with_argument("command", command.trim()),
        );
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_provider_emits_tool_call() {
        let provider = FakeProvider;
        let events = provider.stream(ProviderRequest {
            model: fake_model(),
            messages: vec![ChatMessage::user("tool read Cargo.toml")],
        });

        assert_eq!(
            events,
            vec![ProviderEvent::ToolCall(
                ToolCall::new("fake_tool_read_1", "read")
                    .with_argument("path", "Cargo.toml")
                    .with_argument("offset", "0")
            )]
        );
    }

    #[test]
    fn fake_provider_responds_after_tool_result() {
        let provider = FakeProvider;
        let events = provider.stream(ProviderRequest {
            model: fake_model(),
            messages: vec![
                ChatMessage::user("tool read Cargo.toml"),
                ChatMessage::tool("read output"),
            ],
        });

        assert!(events.contains(&ProviderEvent::TextDelta(
            "fake tool result:\nread output".to_string()
        )));
    }

    #[test]
    fn fake_provider_can_observe_history() {
        let provider = FakeProvider;
        let events = provider.stream(ProviderRequest {
            model: fake_model(),
            messages: vec![
                ChatMessage::user("hello"),
                ChatMessage::assistant("hi"),
                ChatMessage::user("history count"),
            ],
        });

        assert!(events.contains(&ProviderEvent::TextDelta(
            "fake history count: 3".to_string()
        )));
    }

    #[test]
    fn openai_provider_reports_missing_api_key() {
        let provider = OpenAiCompatibleProvider;
        let events = provider.stream(ProviderRequest {
            model: Model::new("openai", "gpt-test", "openai-completions")
                .with_api_key_env("EXGENT_TEST_MISSING_API_KEY"),
            messages: vec![ChatMessage::user("hello")],
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
    fn parses_openai_responses_stream_delta() {
        let deltas = parse_openai_responses_stream_line(
            r#"data: {"type":"response.output_text.delta","delta":"hello"}"#,
        )
        .unwrap();

        assert_eq!(deltas, vec![StreamDelta::Text("hello".to_string())]);
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

    #[test]
    fn ignores_openai_stream_done_marker() {
        assert!(parse_openai_stream_line("data: [DONE]").unwrap().is_empty());
    }

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
    fn anthropic_provider_reports_missing_api_key() {
        let provider = AnthropicMessagesProvider;
        let events = provider.stream(ProviderRequest {
            model: Model::new("anthropic", "claude-test", "anthropic-messages")
                .with_api_key_env("EXGENT_TEST_MISSING_ANTHROPIC_API_KEY"),
            messages: vec![ChatMessage::user("hello")],
        });

        assert!(matches!(
            events.as_slice(),
            [ProviderEvent::Error(message)]
                if message.contains("EXGENT_TEST_MISSING_ANTHROPIC_API_KEY")
        ));
    }

    #[test]
    fn provider_registry_rejects_duplicate_adapters() {
        let mut registry = ProviderRegistry::new();
        registry
            .register("test-adapter", || {
                DynamicProvider::new("test-adapter", FakeProvider)
            })
            .unwrap();

        let error = registry
            .register("test-adapter", || {
                DynamicProvider::new("test-adapter", FakeProvider)
            })
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn built_in_models_include_generated_models_without_fake() {
        let models = built_in_models();

        assert_eq!(models.len(), 931);
        assert!(!models
            .iter()
            .any(|model| model.provider == "fake" || model.adapter == "fake"));
        assert!(models.iter().any(|model| model.provider == "anthropic"
            && model.adapter == "anthropic-messages"
            && model.base_url.as_deref() == Some("https://api.anthropic.com")));
        assert!(models.iter().any(|model| model.provider == "deepseek"
            && model.adapter == "openai-completions"
            && model.base_url.as_deref() == Some("https://api.deepseek.com")));
        assert!(models.iter().any(|model| model.provider == "github-copilot"
            && model.id == "gpt-5.4"
            && model.name.as_deref() == Some("GPT-5.4")
            && model.adapter == "openai-responses"
            && model.reasoning == Some(true)
            && model.context_window == Some(400000)
            && model.max_tokens == Some(128000)
            && model.base_url.as_deref() == Some("https://api.individual.githubcopilot.com")));
        assert!(models
            .iter()
            .any(|model| model.provider == "github-copilot" && model.id == "gpt-5.3-codex"));
        let copilot_sonnet = models
            .iter()
            .find(|model| model.provider == "github-copilot" && model.id == "claude-sonnet-4.5")
            .unwrap();
        assert_eq!(copilot_sonnet.adapter, "anthropic-messages");
        assert_eq!(
            copilot_sonnet.base_url.as_deref(),
            Some("https://api.individual.githubcopilot.com")
        );
        assert_eq!(
            copilot_sonnet.headers.get("Copilot-Integration-Id"),
            Some(&"vscode-chat".to_string())
        );
        assert_eq!(
            copilot_sonnet
                .compat
                .as_ref()
                .and_then(|compat| compat.get("supportsEagerToolInputStreaming"))
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert!(models.iter().any(|model| model.provider == "amazon-bedrock"
            && model.adapter == "bedrock-converse-stream"));
        assert!(models
            .iter()
            .any(|model| model.provider == "google" && model.adapter == "google-generative-ai"));
    }

    #[test]
    fn unsupported_provider_reports_adapter_at_runtime() {
        let registry = ProviderRegistry::builtin();
        let model = Model::new(
            "amazon-bedrock",
            "amazon.nova-lite-v1:0",
            "bedrock-converse-stream",
        );
        let provider = registry.provider_for_model(&model);
        let events = provider.stream(ProviderRequest {
            model,
            messages: vec![ChatMessage::user("hello")],
        });

        assert!(matches!(
            events.as_slice(),
            [ProviderEvent::Error(message)] if message.contains("bedrock-converse-stream")
        ));
    }
}
