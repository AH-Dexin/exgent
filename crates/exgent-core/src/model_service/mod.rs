mod catalog;
mod persistence;

use exgent_ai::{
    discover_available_models, supports_model_discovery, DynamicProvider, Model, ProviderRegistry,
};

use crate::{
    auth::{AuthStore, OAuthCredential},
    config::RuntimeOptions,
    localization::Locale,
    models::{CompatibleModelKind, ModelAvailabilityCache, ModelRegistry},
    settings::{ModelSelection, SettingsStore, ThemeSettings},
};

use catalog::{
    first_selectable_model_index, is_configured_model, is_selectable_model, model_with_auth,
    resolve_initial_model_index,
};

pub use catalog::no_model_configured_message;

pub struct ModelService {
    models: ModelRegistry,
    model_cache: ModelAvailabilityCache,
    auth: AuthStore,
    settings: SettingsStore,
    provider_registry: ProviderRegistry,
    current_model_index: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthProviderInfo {
    pub provider: String,
    pub has_token: bool,
    /// Base URL of the first model registered under this provider, if any.
    /// Used by the auth flow to show users which endpoint their key will hit.
    pub base_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionProviderInfo {
    pub provider: String,
    pub name: String,
    pub has_subscription: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddedModelInfo {
    pub index: usize,
    pub provider: String,
    pub model_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelMenuItem {
    pub index: usize,
    pub provider: String,
    pub id: String,
    pub name: String,
    pub adapter: String,
    pub base_url: Option<String>,
    pub context_window: Option<u64>,
    pub max_tokens: Option<u64>,
    pub is_current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelSettingsItem {
    pub index: usize,
    pub provider: String,
    pub id: String,
    pub name: String,
    pub adapter: String,
    pub is_enabled: bool,
    pub is_custom: bool,
}

impl ModelService {
    pub fn load(options: &RuntimeOptions) -> Result<Self, String> {
        let mut registry = ProviderRegistry::builtin();
        if options.enable_dev_providers {
            registry = registry.with_dev_providers();
        }
        Self::load_with_provider_registry(options, registry)
    }

    pub(crate) fn load_with_provider_registry(
        options: &RuntimeOptions,
        provider_registry: ProviderRegistry,
    ) -> Result<Self, String> {
        let mut models = ModelRegistry::load(options.config_path.as_deref())
            .map_err(|error| format!("failed to load models: {error}"))?;
        let model_cache = ModelAvailabilityCache::load(options.config_path.as_deref())
            .map_err(|error| format!("failed to load model cache: {error}"))?;
        models.apply_availability_cache(&model_cache);
        let auth = AuthStore::load(options.config_path.as_deref())
            .map_err(|error| format!("failed to load auth: {error}"))?;
        let settings = SettingsStore::load(options.config_path.as_deref())
            .map_err(|error| format!("failed to load settings: {error}"))?;
        let current_model_index = resolve_initial_model_index(
            &models,
            &model_cache,
            settings.default_model(),
            &settings,
            &auth,
            provider_registry.subscription_providers(),
        );

        Ok(Self {
            models,
            model_cache,
            auth,
            settings,
            provider_registry,
            current_model_index,
        })
    }

    pub fn current_model_with_provider(&self) -> Result<(Model, DynamicProvider), String> {
        let model = self.current_model_with_auth()?;
        let provider = self.provider_registry.provider_for_model(&model);
        Ok((model, provider))
    }

    pub fn auth_path(&self) -> String {
        self.auth.path().display().to_string()
    }

    pub fn models_path(&self) -> String {
        self.models.path().display().to_string()
    }

    pub fn auth_providers(&self) -> Vec<AuthProviderInfo> {
        let mut providers = Vec::new();
        for model in self.models.models() {
            if model.adapter == "fake"
                || self.is_subscription_provider(&model.provider)
                || providers
                    .iter()
                    .any(|provider: &AuthProviderInfo| provider.provider == model.provider)
            {
                continue;
            }

            providers.push(AuthProviderInfo {
                provider: model.provider.clone(),
                has_token: self.auth.has_api_key(&model.provider),
                base_url: model.base_url.clone(),
            });
        }
        providers
    }

    pub fn subscription_providers(&self) -> Vec<SubscriptionProviderInfo> {
        self.provider_registry
            .subscription_providers()
            .iter()
            .map(|spec| SubscriptionProviderInfo {
                provider: spec.id.clone(),
                name: spec.name.clone(),
                has_subscription: self.auth.has_oauth(&spec.id),
            })
            .collect()
    }

    pub fn set_auth_token(&mut self, provider: &str, token: &str) -> Result<(), String> {
        let previous = self.snapshot();
        self.ensure_known_provider(provider)?;
        self.auth.set_token(provider, token);
        if let Err(error) = self.refresh_provider_model_cache_in_memory(provider, token) {
            self.restore(previous);
            return Err(format!(
                "failed to verify API key and fetch provider models: {error}"
            ));
        }
        self.select_first_model_for_provider_if_present_in_memory(provider)?;
        self.save_auth_cache_and_settings(previous)
    }

    pub fn set_oauth_credential(
        &mut self,
        provider: &str,
        credential: OAuthCredential,
    ) -> Result<(), String> {
        let previous = self.snapshot();
        self.ensure_known_provider(provider)?;
        self.auth.set_oauth(provider, credential);
        self.select_first_model_for_provider_if_present_in_memory(provider)?;
        self.save_auth_and_settings(previous)
    }

    pub fn remove_auth_token(&mut self, provider: &str) -> Result<(), String> {
        let previous = self.snapshot();
        self.auth.remove_token(provider);
        self.model_cache.remove_provider(provider);
        self.current_model_with_provider().map(|_| ())?;
        self.save_auth_cache_and_settings(previous)
    }

    pub fn add_compatible_model(
        &mut self,
        kind: CompatibleModelKind,
        provider: &str,
        model_id: &str,
        base_url: &str,
        api_key: &str,
    ) -> Result<AddedModelInfo, String> {
        let previous = self.snapshot();
        let index = self
            .models
            .add_compatible_model_in_memory(kind, provider, model_id, base_url, None);

        if !api_key.trim().is_empty() {
            self.auth.set_token(provider, api_key.trim());
        }

        if self
            .models
            .get(index)
            .map(|model| is_configured_model(model, &self.auth))
            .unwrap_or(false)
        {
            self.select_model_in_memory(index)?;
        }
        self.save_models_auth_and_settings(previous)?;
        Ok(AddedModelInfo {
            index,
            provider: provider.to_string(),
            model_id: model_id.to_string(),
        })
    }

    pub fn model_label(&self) -> String {
        self.current_model()
            .map(|model| format!("{}/{}", model.provider, model.id))
            .unwrap_or_else(|| "No model configured".to_string())
    }

    pub fn selectable_models(&self) -> Vec<ModelMenuItem> {
        self.models
            .models()
            .iter()
            .enumerate()
            .filter(|(index, model)| {
                is_selectable_model(
                    &self.models,
                    &self.model_cache,
                    &self.settings,
                    &self.auth,
                    *index,
                    model,
                )
            })
            .map(|(index, model)| ModelMenuItem {
                index,
                provider: model.provider.clone(),
                id: model.id.clone(),
                name: model.display_name().to_string(),
                adapter: model.adapter.clone(),
                base_url: model.base_url.clone(),
                context_window: model.context_window,
                max_tokens: model.max_tokens,
                is_current: Some(index) == self.current_model_index,
            })
            .collect()
    }

    pub fn select_model(&mut self, index: usize) -> Result<(), String> {
        let previous = self.snapshot();
        self.select_model_in_memory(index)?;
        self.save_settings(previous)
    }

    pub fn model_settings_items(&self) -> Vec<ModelSettingsItem> {
        self.models
            .models()
            .iter()
            .enumerate()
            .filter(|(index, model)| {
                model.provider != "fake"
                    && catalog::is_available_model(&self.models, &self.model_cache, *index, model)
                    && is_configured_model(model, &self.auth)
            })
            .map(|(index, model)| ModelSettingsItem {
                index,
                provider: model.provider.clone(),
                id: model.id.clone(),
                name: model.display_name().to_string(),
                adapter: model.adapter.clone(),
                is_enabled: self.settings.is_model_enabled(model),
                is_custom: self.models.is_custom_model(index),
            })
            .collect()
    }

    pub fn delete_model(&mut self, index: usize) -> Result<String, String> {
        let previous = self.snapshot();
        let Some(model) = self.models.get(index).cloned() else {
            return Err(format!("model selection out of range: {index}"));
        };

        if !self.models.is_custom_model(index) {
            self.auth.remove_token(&model.provider);
            self.model_cache.remove_provider(&model.provider);
            self.current_model_index = first_selectable_model_index(
                &self.models,
                &self.model_cache,
                &self.settings,
                &self.auth,
            );
            self.save_auth_cache_and_settings(previous)?;
            return Ok(format!("removed auth for {}/{}", model.provider, model.id));
        }

        let removed = self
            .models
            .remove_custom_model_in_memory(index)
            .map_err(|error| format!("failed to delete model: {error}"))?;
        self.current_model_index = first_selectable_model_index(
            &self.models,
            &self.model_cache,
            &self.settings,
            &self.auth,
        );

        self.save_models_and_settings(previous)?;
        Ok(format!("{}/{}", removed.provider, removed.id))
    }

    pub fn prompt_display_enabled(&self) -> bool {
        self.settings.prompt_display_enabled()
    }

    pub fn set_prompt_display_enabled(&mut self, enabled: bool) -> Result<(), String> {
        let previous = self.snapshot();
        self.settings.set_prompt_display_enabled(enabled);
        self.save_settings(previous)
    }

    pub fn locale(&self) -> Locale {
        self.settings.locale()
    }

    pub fn locale_setting(&self) -> &str {
        self.settings.locale_setting()
    }

    pub fn set_locale(&mut self, locale: &str) -> Result<(), String> {
        let previous = self.snapshot();
        self.settings.set_locale(locale)?;
        self.save_settings(previous)
    }

    pub fn theme(&self) -> ThemeSettings {
        self.settings.theme()
    }

    pub fn set_theme(&mut self, theme: ThemeSettings) -> Result<(), String> {
        let previous = self.snapshot();
        self.settings.set_theme(theme);
        self.save_settings(previous)
    }

    pub fn keybindings(&self) -> crate::settings::KeyBindings {
        self.settings.keybindings().clone()
    }

    pub fn set_keybindings(
        &mut self,
        keybindings: crate::settings::KeyBindings,
    ) -> Result<(), String> {
        let previous = self.snapshot();
        self.settings.set_keybindings(keybindings);
        self.save_settings(previous)
    }

    pub fn set_enabled_model_indices(&mut self, enabled_indices: &[usize]) -> Result<(), String> {
        let previous = self.snapshot();
        let mut selections_by_provider =
            std::collections::BTreeMap::<String, Vec<ModelSelection>>::new();

        for (index, model) in self.models.models().iter().enumerate() {
            if model.provider == "fake"
                || !catalog::is_available_model(&self.models, &self.model_cache, index, model)
                || !is_configured_model(model, &self.auth)
            {
                continue;
            }
            selections_by_provider
                .entry(model.provider.clone())
                .or_default();
            if enabled_indices.contains(&index) {
                selections_by_provider
                    .entry(model.provider.clone())
                    .or_default()
                    .push(ModelSelection::from_model(model));
            }
        }

        for (provider, selections) in selections_by_provider {
            self.settings.set_enabled_models(&provider, selections);
        }

        self.current_model_index = self.current_model_index.and_then(|index| {
            self.models
                .get(index)
                .filter(|model| {
                    catalog::is_available_model(&self.models, &self.model_cache, index, model)
                        && self.settings.is_model_enabled(model)
                })
                .map(|_| index)
        });
        if self.current_model_index.is_none() {
            self.current_model_index = self
                .models
                .models()
                .iter()
                .enumerate()
                .find(|(index, model)| {
                    is_selectable_model(
                        &self.models,
                        &self.model_cache,
                        &self.settings,
                        &self.auth,
                        *index,
                        model,
                    )
                })
                .map(|(index, _)| index);
        }

        self.save_settings(previous)
    }

    fn select_model_in_memory(&mut self, index: usize) -> Result<(), String> {
        let model_without_auth = self
            .models
            .get(index)
            .ok_or_else(|| format!("model selection out of range: {index}"))?;
        if !is_configured_model(model_without_auth, &self.auth) {
            return Err(format!(
                "model is not configured: {}/{}. Please enter /auth to configure a model.",
                model_without_auth.provider, model_without_auth.id
            ));
        }
        if !catalog::is_available_model(&self.models, &self.model_cache, index, model_without_auth)
        {
            return Err(format!(
                "model is not available for this account: {}/{}. Refresh /auth to update available models.",
                model_without_auth.provider, model_without_auth.id
            ));
        }
        if !self.settings.is_model_enabled(model_without_auth) {
            return Err(format!(
                "model is hidden in settings: {}/{}. Please enter /settings model to enable it.",
                model_without_auth.provider, model_without_auth.id
            ));
        }

        let selection = ModelSelection::from_model(model_without_auth);
        self.current_model_index = Some(index);
        self.settings.set_default_model(selection);
        Ok(())
    }

    fn select_first_model_for_provider_if_present_in_memory(
        &mut self,
        provider: &str,
    ) -> Result<(), String> {
        if let Some(index) = self
            .models
            .models()
            .iter()
            .enumerate()
            .find_map(|(index, model)| {
                (model.provider == provider
                    && catalog::is_available_model(&self.models, &self.model_cache, index, model)
                    && is_configured_model(model, &self.auth)
                    && self.settings.is_model_enabled(model))
                .then_some(index)
            })
        {
            self.select_model_in_memory(index)
        } else {
            self.current_model_with_provider().map(|_| ())
        }
    }

    fn refresh_provider_model_cache_in_memory(
        &mut self,
        provider: &str,
        token: &str,
    ) -> Result<(), String> {
        if !supports_model_discovery(provider) {
            return Ok(());
        }

        let base_url = self.discovery_base_url(provider);
        let models = discover_available_models(provider, token, base_url.as_deref())?;
        if models.is_empty() {
            return Err(format!("provider returned no usable models for {provider}"));
        }

        self.model_cache.set_provider_models(provider, models);
        self.models.apply_availability_cache(&self.model_cache);
        Ok(())
    }

    fn discovery_base_url(&self, provider: &str) -> Option<String> {
        self.models
            .models()
            .iter()
            .find(|model| model.provider == provider)
            .and_then(|model| model.base_url.clone())
    }

    pub fn current_model(&self) -> Option<&Model> {
        self.current_model_index
            .and_then(|index| self.models.get(index))
    }

    fn current_model_with_auth(&self) -> Result<Model, String> {
        self.current_model()
            .map(|model| model_with_auth(model, &self.auth))
            .ok_or_else(|| no_model_configured_message().to_string())
    }

    fn ensure_known_provider(&self, provider: &str) -> Result<(), String> {
        if self
            .models
            .models()
            .iter()
            .any(|model| model.provider == provider)
        {
            Ok(())
        } else {
            Err(format!("unknown provider: {provider}"))
        }
    }

    fn is_subscription_provider(&self, provider: &str) -> bool {
        self.provider_registry
            .subscription_providers()
            .iter()
            .any(|spec| spec.id == provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_selected_model_across_service_restarts() {
        let dir = test_dir("saves_selected_model_across_service_restarts");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("models.json"),
            r#"{
              "models": [
                {
                  "provider": "test-provider",
                  "id": "test-model",
                  "adapter": "openai-completions",
                  "base_url": "https://example.test/v1"
                }
              ]
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("settings.json"),
            r#"{"default_model":{"provider":"test-provider","id":"test-model","adapter":"openai-completions"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("auth.json"),
            r#"{"credentials":{"test-provider":{"type":"api_key","key":"test-token"}}}"#,
        )
        .unwrap();

        let mut service = ModelService::load(&RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        })
        .unwrap();
        let test_model_index = service
            .models
            .models()
            .iter()
            .position(|model| model.provider == "test-provider")
            .unwrap();

        service.select_model(test_model_index).unwrap();
        let loaded = ModelService::load(&RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        })
        .unwrap();

        assert_eq!(loaded.model_label(), "test-provider/test-model");
        assert!(loaded
            .selectable_models()
            .iter()
            .all(|model| model.provider == "test-provider"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn selectable_models_include_all_configured_providers() {
        let dir = test_dir("selectable_models_include_all_configured_providers");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("models.json"),
            r#"{
              "models": [
                {
                  "provider": "api-provider",
                  "id": "api-model",
                  "adapter": "openai-completions",
                  "base_url": "https://api.example.test/v1"
                },
                {
                  "provider": "oauth-provider",
                  "id": "oauth-model",
                  "adapter": "openai-responses",
                  "base_url": "https://oauth.example.test/v1"
                },
                {
                  "provider": "missing-provider",
                  "id": "missing-model",
                  "adapter": "openai-completions",
                  "base_url": "https://missing.example.test/v1"
                }
              ]
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("auth.json"),
            r#"{
              "credentials": {
                "api-provider": {"type":"api_key","key":"api-token"},
                "oauth-provider": {
                  "type":"o_auth",
                  "access":"oauth-access",
                  "refresh":"oauth-refresh",
                  "expires":0
                }
              }
            }"#,
        )
        .unwrap();

        let service = ModelService::load(&RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        })
        .unwrap();

        let models = service.selectable_models();
        assert!(models
            .iter()
            .any(|model| model.provider == "api-provider" && model.id == "api-model"));
        assert!(models
            .iter()
            .any(|model| model.provider == "oauth-provider" && model.id == "oauth-model"));
        assert!(!models
            .iter()
            .any(|model| model.provider == "missing-provider"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_settings_filter_selectable_models() {
        let dir = test_dir("model_settings_filter_selectable_models");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("models.json"),
            r#"{
              "models": [
                {
                  "provider": "test-provider",
                  "id": "visible-model",
                  "adapter": "openai-completions",
                  "base_url": "https://example.test/v1"
                },
                {
                  "provider": "test-provider",
                  "id": "hidden-model",
                  "adapter": "openai-completions",
                  "base_url": "https://example.test/v1"
                }
              ]
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("auth.json"),
            r#"{"credentials":{"test-provider":{"type":"api_key","key":"test-token"}}}"#,
        )
        .unwrap();

        let mut service = ModelService::load(&RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        })
        .unwrap();
        let visible_index = service
            .model_settings_items()
            .iter()
            .find(|model| model.id == "visible-model")
            .unwrap()
            .index;

        service.set_enabled_model_indices(&[visible_index]).unwrap();

        let models = service.selectable_models();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "visible-model");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_cache_filters_provider_models() {
        let dir = test_dir("model_cache_filters_provider_models");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("auth.json"),
            r#"{"credentials":{"anthropic":{"type":"api_key","key":"test-token"}}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("model_cache.json"),
            r#"{
              "providers": {
                "anthropic": {
                  "fetched_at": "2026-05-26T00:00:00Z",
                  "models": [
                    { "id": "claude-sonnet-4-5", "name": "Claude Sonnet 4.5" }
                  ]
                }
              }
            }"#,
        )
        .unwrap();

        let options = RuntimeOptions {
            config_path: Some(dir.to_str().unwrap().to_string()),
            ..RuntimeOptions::default()
        };
        let service = ModelService::load(&options).unwrap();
        let models = service.selectable_models();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].provider, "anthropic");
        assert_eq!(models[0].id, "claude-sonnet-4-5");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn adds_anthropic_compatible_model_with_api_key() {
        let dir = test_dir("adds_anthropic_compatible_model_with_api_key");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut service = ModelService::load(&RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        })
        .unwrap();

        let info = service
            .add_compatible_model(
                CompatibleModelKind::Anthropic,
                "anthropic-proxy",
                "claude-sonnet-4-5",
                "https://api.anthropic.com",
                "test-token",
            )
            .unwrap();

        assert_eq!(info.provider, "anthropic-proxy");
        assert_eq!(service.model_label(), "anthropic-proxy/claude-sonnet-4-5");
        let loaded = ModelService::load(&RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        })
        .unwrap();
        assert!(loaded.selectable_models().iter().any(|model| {
            model.provider == "anthropic-proxy" && model.adapter == "anthropic-messages"
        }));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn adds_google_compatible_model_with_api_key() {
        let dir = test_dir("adds_google_compatible_model_with_api_key");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut service = ModelService::load(&RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        })
        .unwrap();

        let info = service
            .add_compatible_model(
                CompatibleModelKind::Google,
                "google-proxy",
                "gemini-test",
                "https://generativelanguage.googleapis.com/v1beta",
                "test-token",
            )
            .unwrap();

        assert_eq!(info.provider, "google-proxy");
        assert_eq!(service.model_label(), "google-proxy/gemini-test");
        let loaded = ModelService::load(&RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        })
        .unwrap();
        assert!(loaded.selectable_models().iter().any(|model| {
            model.provider == "google-proxy" && model.adapter == "google-generative-ai"
        }));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_built_in_model_removes_provider_auth() {
        let dir = test_dir("deleting_built_in_model_removes_provider_auth");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("auth.json"),
            r#"{"credentials":{"anthropic":{"type":"api_key","key":"test-token"}}}"#,
        )
        .unwrap();

        let mut service = ModelService::load(&RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        })
        .unwrap();
        let anthropic_index = service
            .model_settings_items()
            .iter()
            .find(|model| model.provider == "anthropic")
            .unwrap()
            .index;

        let message = service.delete_model(anthropic_index).unwrap();
        let auth = AuthStore::load(Some(dir.to_str().unwrap())).unwrap();

        assert!(message.starts_with("removed auth for anthropic/"));
        assert!(!auth.has_api_key("anthropic"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_auth_for_unknown_provider() {
        let dir = test_dir("rejects_auth_for_unknown_provider");
        let _ = std::fs::remove_dir_all(&dir);

        let mut service = ModelService::load(&RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        })
        .unwrap();

        let error = service
            .set_auth_token("missing-provider", "test-token")
            .unwrap_err();

        assert_eq!(error, "unknown provider: missing-provider");
        assert!(!dir.join("auth.json").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn test_dir(name: &str) -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("exgent_model_service_{name}_{stamp}"))
    }
}
