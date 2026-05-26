mod cache;
mod config;
mod custom;
mod registry;

pub use cache::ModelAvailabilityCache;
pub use custom::CompatibleModelKind;
pub use registry::ModelRegistry;

#[cfg(test)]
mod tests {
    use std::{fs, io, path::PathBuf};

    use super::*;
    use crate::settings::ModelSelection;

    #[test]
    fn loads_generated_models_by_default_without_fake_model() {
        let dir = test_dir("loads_generated_models_by_default_without_fake_model");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let registry = ModelRegistry::load(Some(dir.to_str().unwrap())).unwrap();

        assert!(!registry
            .models()
            .iter()
            .any(|model| model.provider == "fake" || model.adapter == "fake"));
        assert!(registry
            .models()
            .iter()
            .any(|model| model.provider == "anthropic" && model.adapter == "anthropic-messages"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_configured_openai_compatible_models() {
        let dir = test_dir("loads_configured_openai_compatible_models");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("models.json"),
            r#"{
              "models": [
                {
                  "provider": "deepseek",
                  "id": "deepseek-chat",
                  "adapter": "openai-completions",
                  "base_url": "https://api.deepseek.com/v1",
                  "api_key_env": "DEEPSEEK_API_KEY"
                }
              ]
            }"#,
        )
        .unwrap();

        let registry = ModelRegistry::load(Some(dir.to_str().unwrap())).unwrap();

        let deepseek = registry
            .models()
            .iter()
            .find(|model| model.provider == "deepseek" && model.id == "deepseek-chat")
            .unwrap();
        assert_eq!(deepseek.adapter, "openai-completions");
        assert_eq!(
            deepseek.base_url.as_deref(),
            Some("https://api.deepseek.com/v1")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn adds_openai_compatible_model_to_config() {
        let dir = test_dir("adds_openai_compatible_model_to_config");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut registry = ModelRegistry::load(Some(dir.to_str().unwrap())).unwrap();
        let index = registry.add_compatible_model_in_memory(
            CompatibleModelKind::OpenAi,
            "deepseek",
            "deepseek-chat",
            "https://api.deepseek.com/v1",
            Some("DEEPSEEK_API_KEY".to_string()),
        );
        registry.save().unwrap();

        assert!(index > 0);
        let loaded = ModelRegistry::load(Some(dir.to_str().unwrap())).unwrap();
        assert!(loaded
            .models()
            .iter()
            .any(|model| model.provider == "deepseek"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn adds_anthropic_compatible_model_to_config() {
        let dir = test_dir("adds_anthropic_compatible_model_to_config");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut registry = ModelRegistry::load(Some(dir.to_str().unwrap())).unwrap();
        let index = registry.add_compatible_model_in_memory(
            CompatibleModelKind::Anthropic,
            "anthropic-proxy",
            "claude-sonnet-4-5",
            "https://api.anthropic.com",
            Some("ANTHROPIC_API_KEY".to_string()),
        );
        registry.save().unwrap();

        assert!(index > 0);
        let loaded = ModelRegistry::load(Some(dir.to_str().unwrap())).unwrap();
        let model = loaded
            .models()
            .iter()
            .find(|model| model.provider == "anthropic-proxy")
            .unwrap();
        assert_eq!(model.adapter, "anthropic-messages");
        assert_eq!(model.id, "claude-sonnet-4-5");
        assert_eq!(model.base_url.as_deref(), Some("https://api.anthropic.com"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn adds_google_compatible_model_to_config() {
        let dir = test_dir("adds_google_compatible_model_to_config");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut registry = ModelRegistry::load(Some(dir.to_str().unwrap())).unwrap();
        let index = registry.add_compatible_model_in_memory(
            CompatibleModelKind::Google,
            "google-proxy",
            "gemini-test",
            "https://generativelanguage.googleapis.com/v1beta",
            Some("GEMINI_API_KEY".to_string()),
        );
        registry.save().unwrap();

        assert!(index > 0);
        let loaded = ModelRegistry::load(Some(dir.to_str().unwrap())).unwrap();
        let model = loaded
            .models()
            .iter()
            .find(|model| model.provider == "google-proxy")
            .unwrap();
        assert_eq!(model.adapter, "google-generative-ai");
        assert_eq!(model.id, "gemini-test");
        assert_eq!(
            model.base_url.as_deref(),
            Some("https://generativelanguage.googleapis.com/v1beta")
        );
        assert_eq!(model.api_key_env.as_deref(), Some("GEMINI_API_KEY"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deletes_custom_model_without_removing_built_in_models() {
        let dir = test_dir("deletes_custom_model_without_removing_built_in_models");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut registry = ModelRegistry::load(Some(dir.to_str().unwrap())).unwrap();
        let index = registry.add_compatible_model_in_memory(
            CompatibleModelKind::OpenAi,
            "custom-provider",
            "custom-model",
            "https://example.test/v1",
            None,
        );
        registry.save().unwrap();

        let removed = registry.remove_custom_model_in_memory(index).unwrap();
        registry.save().unwrap();

        assert_eq!(removed.provider, "custom-provider");
        assert!(!registry
            .models()
            .iter()
            .any(|model| model.provider == "custom-provider"));
        assert!(registry
            .models()
            .iter()
            .any(|model| model.provider == "anthropic"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_deleting_built_in_model() {
        let dir = test_dir("rejects_deleting_built_in_model");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut registry = ModelRegistry::load(Some(dir.to_str().unwrap())).unwrap();
        let index = registry
            .models()
            .iter()
            .position(|model| model.provider == "anthropic")
            .unwrap();

        let error = registry.remove_custom_model_in_memory(index).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_model_by_settings_selection() {
        let dir = test_dir("finds_model_by_settings_selection");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let registry = ModelRegistry::load(Some(dir.to_str().unwrap())).unwrap();
        let index = registry
            .models()
            .iter()
            .position(|model| model.provider == "anthropic")
            .unwrap();
        let selection = ModelSelection::from_model(&registry.models()[index]);

        assert_eq!(registry.model_index_for_selection(&selection), Some(index));

        let _ = fs::remove_dir_all(&dir);
    }

    fn test_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("exgent_models_{name}_{stamp}"))
    }
}
