use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{Model, ModelCost};

const BUILTIN_PROVIDER_ADAPTERS: &[&str] = &[
    "openai-completions",
    "openai-responses",
    "openai-codex-responses",
    "anthropic-messages",
    "google-generative-ai",
];

pub(crate) fn is_builtin_provider_adapter(adapter: &str) -> bool {
    BUILTIN_PROVIDER_ADAPTERS.contains(&adapter)
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
        .filter(|model| is_builtin_provider_adapter(&model.adapter))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_models_include_generated_models_without_fake() {
        let models = built_in_models();

        assert!(models.len() > 745);
        assert!(!models
            .iter()
            .any(|model| model.provider == "fake" || model.adapter == "fake"));
        assert!(models
            .iter()
            .all(|model| is_builtin_provider_adapter(&model.adapter)));
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
        assert!(models.iter().any(|model| {
            model.provider == "openai-codex"
                && model.id == "gpt-5.5"
                && model.adapter == "openai-codex-responses"
                && model.base_url.as_deref() == Some("https://chatgpt.com/backend-api")
        }));
        assert!(models.iter().any(|model| {
            model.provider == "google"
                && model.id == "gemini-2.5-flash"
                && model.adapter == "google-generative-ai"
                && model.api_key_env.as_deref() == Some("GEMINI_API_KEY")
                && model.base_url.as_deref()
                    == Some("https://generativelanguage.googleapis.com/v1beta")
        }));
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
        assert!(!models.iter().any(|model| {
            matches!(
                model.adapter.as_str(),
                "azure-openai-responses"
                    | "bedrock-converse-stream"
                    | "google-vertex"
                    | "mistral-conversations"
            )
        }));
    }
}
