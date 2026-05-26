//! Provider and adapter primitives for exgent.

mod llm_convert;
mod model_discovery;
mod models;
mod providers;
mod registry;
mod types;

pub use model_discovery::{discover_available_models, supports_model_discovery, DiscoveredModel};
pub use models::{built_in_models, generated_models};
pub use providers::{
    fake_model, AnthropicMessagesProvider, FakeProvider, GoogleGenerativeAiProvider,
    OpenAiCodexResponsesProvider, OpenAiCompatibleProvider, OpenAiResponsesProvider,
    UnsupportedProvider,
};
pub use registry::{DynamicProvider, ProviderRegistry, SubscriptionProvider};
pub use types::{
    AssistantMessage, ChatMessage, MessageRole, Model, ModelCost, ProviderAdapter, ProviderEvent,
    ProviderRequest, TokenUsage, ToolArguments, ToolCall, ToolDefinition, ToolExecutionMode,
};
