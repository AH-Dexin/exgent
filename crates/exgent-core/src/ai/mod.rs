mod http;
pub(crate) mod llm_convert;
mod model_discovery;
mod models;
mod providers;
mod registry;
mod types;

pub use http::shared_blocking_client;
pub use model_discovery::{discover_available_models, supports_model_discovery, DiscoveredModel};
pub use models::built_in_models;
#[cfg(test)]
pub use providers::fake_model;
pub use providers::{
    AnthropicMessagesProvider, FakeProvider, GoogleGenerativeAiProvider,
    OpenAiCodexResponsesProvider, OpenAiCompatibleProvider, OpenAiResponsesProvider,
    UnsupportedProvider,
};
pub use registry::{DynamicProvider, ProviderRegistry, SubscriptionProvider};
pub use types::{
    AssistantMessage, ChatMessage, ImageContent, MessageRole, Model, ModelCost, ProviderAdapter,
    ProviderEvent, ProviderRequest, TokenUsage, ToolArguments, ToolCall, ToolDefinition,
    ToolExecutionMode,
};
