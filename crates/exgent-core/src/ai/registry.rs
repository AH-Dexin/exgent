use std::{collections::BTreeMap, fmt, io, sync::Arc};

use super::{
    AnthropicMessagesProvider, FakeProvider, GoogleGenerativeAiProvider, Model,
    OpenAiCodexResponsesProvider, OpenAiCompatibleProvider, OpenAiResponsesProvider,
    ProviderAdapter, ProviderEvent, ProviderRequest, UnsupportedProvider,
};

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

    fn stream_events_cancellable(
        &self,
        request: ProviderRequest,
        should_cancel: &dyn Fn() -> bool,
        emit: &mut dyn FnMut(ProviderEvent),
    ) {
        self.adapter
            .stream_events_cancellable(request, should_cancel, emit);
    }
}

#[derive(Clone)]
pub struct ProviderRegistry {
    providers: BTreeMap<String, DynamicProvider>,
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
            .register(DynamicProvider::new("openai-completions", OpenAiCompatibleProvider))
            .expect("built-in provider adapters must be unique");
        registry
            .register(DynamicProvider::new("openai-responses", OpenAiResponsesProvider))
            .expect("built-in provider adapters must be unique");
        registry
            .register(DynamicProvider::new("openai-codex-responses", OpenAiCodexResponsesProvider))
            .expect("built-in provider adapters must be unique");
        registry
            .register(DynamicProvider::new("anthropic-messages", AnthropicMessagesProvider))
            .expect("built-in provider adapters must be unique");
        registry
            .register(DynamicProvider::new("google-generative-ai", GoogleGenerativeAiProvider))
            .expect("built-in provider adapters must be unique");
        registry
            .register_subscription_provider("openai-codex", "ChatGPT Plus/Pro (Codex Subscription)")
            .expect("built-in subscription providers must be unique");
        registry
            .register_subscription_provider("anthropic", "Anthropic (Claude Pro/Max)")
            .expect("built-in subscription providers must be unique");
        registry
            .register_subscription_provider("github-copilot", "GitHub Copilot")
            .expect("built-in subscription providers must be unique");
        registry
    }

    pub fn with_dev_providers(mut self) -> Self {
        if !self.providers.contains_key("fake") {
            self.register(DynamicProvider::new("fake", FakeProvider))
                .expect("dev provider adapters must be unique");
        }
        self
    }

    pub fn new() -> Self {
        Self {
            providers: BTreeMap::new(),
            subscription_providers: Vec::new(),
        }
    }

    pub fn register(&mut self, provider: DynamicProvider) -> io::Result<()> {
        let adapter = provider.adapter_name.clone();
        if self.providers.contains_key(&adapter) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("provider adapter already registered: {adapter}"),
            ));
        }

        self.providers.insert(adapter, provider);
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
        self.providers
            .get(&model.adapter)
            .cloned()
            .unwrap_or_else(|| {
                DynamicProvider::new(
                    model.adapter.clone(),
                    UnsupportedProvider::new(model.adapter.clone()),
                )
            })
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{fake_model, ChatMessage, ProviderEvent, ProviderRequest};

    #[test]
    fn provider_registry_rejects_duplicate_adapters() {
        let mut registry = ProviderRegistry::new();
        registry
            .register(DynamicProvider::new("test-adapter", FakeProvider))
            .unwrap();

        let error = registry
            .register(DynamicProvider::new("test-adapter", FakeProvider))
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn builtin_registry_does_not_enable_fake_provider() {
        let registry = ProviderRegistry::builtin();
        let model = fake_model();
        let provider = registry.provider_for_model(&model);
        let events = provider.stream(ProviderRequest {
            model,
            messages: vec![ChatMessage::user("hello")],
            tools: Vec::new(),
        });

        assert!(matches!(
            events.as_slice(),
            [ProviderEvent::Error(message)] if message.contains("unsupported adapter: fake")
        ));
    }

    #[test]
    fn dev_registry_can_enable_fake_provider() {
        let registry = ProviderRegistry::builtin().with_dev_providers();
        let model = fake_model();
        let provider = registry.provider_for_model(&model);
        let events = provider.stream(ProviderRequest {
            model,
            messages: vec![ChatMessage::user("hello")],
            tools: Vec::new(),
        });

        assert!(events.contains(&ProviderEvent::TextDelta(
            "fake response: hello".to_string()
        )));
    }

    #[test]
    fn subscription_providers_only_include_supported_adapters() {
        let registry = ProviderRegistry::builtin();
        let providers = registry.subscription_providers();

        assert!(providers.iter().any(|provider| provider.id == "anthropic"));
        assert!(providers
            .iter()
            .any(|provider| provider.id == "github-copilot"));
        assert!(providers
            .iter()
            .any(|provider| provider.id == "openai-codex"));
    }

    #[test]
    fn builtin_registry_includes_google_generative_ai() {
        let registry = ProviderRegistry::builtin();
        let model = Model::new("google", "gemini-test", "google-generative-ai")
            .with_api_key_env("EXGENT_TEST_MISSING_GEMINI_API_KEY");
        let provider = registry.provider_for_model(&model);
        let events = provider.stream(ProviderRequest {
            model,
            messages: vec![ChatMessage::user("hello")],
            tools: Vec::new(),
        });

        assert!(matches!(
            events.as_slice(),
            [ProviderEvent::Error(message)] if message.contains("EXGENT_TEST_MISSING_GEMINI_API_KEY")
        ));
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
            tools: Vec::new(),
        });

        assert!(matches!(
            events.as_slice(),
            [ProviderEvent::Error(message)] if message.contains("bedrock-converse-stream")
        ));
    }
}
