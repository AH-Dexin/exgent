use std::{collections::BTreeMap, fs, io, path::PathBuf};

use exgent_ai::{built_in_models, Model, ModelCost};
use serde::{Deserialize, Serialize};

use crate::{config::config_file, persistence::write_json_pretty, settings::ModelSelection};

#[derive(Clone, Debug, PartialEq)]
pub struct ModelRegistry {
    path: PathBuf,
    models: Vec<Model>,
    custom_models: Vec<Model>,
}

impl ModelRegistry {
    pub fn load(config_path: Option<&str>) -> io::Result<Self> {
        let mut models = built_in_models();
        let path = models_config_path(config_path);
        let mut custom_models = Vec::new();
        if path.exists() {
            let config = fs::read_to_string(&path)?;
            let parsed: ModelsConfig = serde_json::from_str(&config).map_err(invalid_data)?;
            for model in parsed.models.into_iter().map(ConfigModel::into_model) {
                merge_model(&mut models, model.clone());
                merge_model(&mut custom_models, model);
            }
        }

        Ok(Self {
            path,
            models,
            custom_models,
        })
    }

    pub fn models(&self) -> &[Model] {
        &self.models
    }

    pub fn get(&self, index: usize) -> Option<&Model> {
        self.models.get(index)
    }

    pub fn model_index_for_selection(&self, selected: &ModelSelection) -> Option<usize> {
        self.models
            .iter()
            .position(|model| selected.matches_model(model))
    }

    pub fn first_model_index_for_provider(&self, provider: &str) -> Option<usize> {
        self.models
            .iter()
            .position(|model| model.provider == provider)
    }

    pub fn add_openai_compatible_model(
        &mut self,
        provider: impl Into<String>,
        model_id: impl Into<String>,
        base_url: impl Into<String>,
        api_key_env: Option<String>,
    ) -> io::Result<usize> {
        let index =
            self.add_openai_compatible_model_in_memory(provider, model_id, base_url, api_key_env);
        self.save()?;
        Ok(index)
    }

    pub fn add_openai_compatible_model_in_memory(
        &mut self,
        provider: impl Into<String>,
        model_id: impl Into<String>,
        base_url: impl Into<String>,
        api_key_env: Option<String>,
    ) -> usize {
        let model = ConfigModel {
            provider: provider.into(),
            id: model_id.into(),
            name: None,
            adapter: "openai-completions".to_string(),
            base_url: Some(base_url.into()),
            api_key_env,
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
        .into_model();

        if let Some(index) = self.models.iter().position(|existing| {
            existing.provider == model.provider
                && existing.id == model.id
                && existing.adapter == model.adapter
        }) {
            merge_model_override(&mut self.models[index], model.clone());
            merge_model(&mut self.custom_models, model);
            return index;
        }

        self.models.push(model.clone());
        merge_model(&mut self.custom_models, model);
        self.models.len() - 1
    }

    pub fn is_custom_model(&self, index: usize) -> bool {
        self.models
            .get(index)
            .map(|model| {
                self.custom_models
                    .iter()
                    .any(|custom| same_model(custom, model))
            })
            .unwrap_or(false)
    }

    pub fn remove_custom_model(&mut self, index: usize) -> io::Result<Model> {
        let model = self.models.get(index).cloned().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "model index out of range")
        })?;
        let Some(custom_index) = self
            .custom_models
            .iter()
            .position(|custom| same_model(custom, &model))
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "cannot delete built-in model: {}/{}",
                    model.provider, model.id
                ),
            ));
        };

        self.custom_models.remove(custom_index);
        self.rebuild_models();
        self.save()?;
        Ok(model)
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn save(&self) -> io::Result<()> {
        let file = ModelsConfig {
            models: self
                .custom_models
                .iter()
                .map(ConfigModel::from_model)
                .collect(),
        };
        write_json_pretty(&self.path, &file)
    }

    fn rebuild_models(&mut self) {
        self.models = built_in_models();
        for model in self.custom_models.clone() {
            merge_model(&mut self.models, model);
        }
    }
}

fn same_model(left: &Model, right: &Model) -> bool {
    left.provider == right.provider && left.id == right.id && left.adapter == right.adapter
}

fn merge_model(models: &mut Vec<Model>, model: Model) {
    if let Some(index) = models.iter().position(|existing| {
        existing.provider == model.provider
            && existing.id == model.id
            && existing.adapter == model.adapter
    }) {
        merge_model_override(&mut models[index], model);
    } else {
        models.push(model);
    }
}

fn merge_model_override(existing: &mut Model, model: Model) {
    existing.provider = model.provider;
    existing.id = model.id;
    existing.adapter = model.adapter;
    if model.name.is_some() {
        existing.name = model.name;
    }
    if model.base_url.is_some() {
        existing.base_url = model.base_url;
    }
    if model.api_key_env.is_some() {
        existing.api_key_env = model.api_key_env;
    }
    if !model.headers.is_empty() {
        existing.headers = model.headers;
    }
    if model.compat.is_some() {
        existing.compat = model.compat;
    }
    if model.reasoning.is_some() {
        existing.reasoning = model.reasoning;
    }
    if !model.thinking_level_map.is_empty() {
        existing.thinking_level_map = model.thinking_level_map;
    }
    if !model.input.is_empty() {
        existing.input = model.input;
    }
    if !model.output.is_empty() {
        existing.output = model.output;
    }
    if model.cost.is_some() {
        existing.cost = model.cost;
    }
    if model.context_window.is_some() {
        existing.context_window = model.context_window;
    }
    if model.max_tokens.is_some() {
        existing.max_tokens = model.max_tokens;
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct ModelsConfig {
    models: Vec<ConfigModel>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ConfigModel {
    provider: String,
    id: String,
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

impl ConfigModel {
    fn from_model(model: &Model) -> Self {
        Self {
            provider: model.provider.clone(),
            id: model.id.clone(),
            name: model.name.clone(),
            adapter: model.adapter.clone(),
            base_url: model.base_url.clone(),
            api_key_env: model.api_key_env.clone(),
            headers: model.headers.clone(),
            compat: model.compat.clone(),
            reasoning: model.reasoning,
            thinking_level_map: model.thinking_level_map.clone(),
            input: model.input.clone(),
            output: model.output.clone(),
            cost: model.cost.clone(),
            context_window: model.context_window,
            max_tokens: model.max_tokens,
        }
    }

    fn into_model(self) -> Model {
        let mut model = Model::new(self.provider, self.id, self.adapter);
        model.name = self.name;
        model.base_url = self.base_url;
        model.api_key_env = self.api_key_env;
        model.headers = self.headers;
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

fn models_config_path(config_path: Option<&str>) -> PathBuf {
    config_file(config_path, "models.json")
}

fn invalid_data(error: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let index = registry
            .add_openai_compatible_model(
                "deepseek",
                "deepseek-chat",
                "https://api.deepseek.com/v1",
                Some("DEEPSEEK_API_KEY".to_string()),
            )
            .unwrap();

        assert!(index > 0);
        let loaded = ModelRegistry::load(Some(dir.to_str().unwrap())).unwrap();
        assert!(loaded
            .models()
            .iter()
            .any(|model| model.provider == "deepseek"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deletes_custom_model_without_removing_built_in_models() {
        let dir = test_dir("deletes_custom_model_without_removing_built_in_models");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut registry = ModelRegistry::load(Some(dir.to_str().unwrap())).unwrap();
        let index = registry.add_openai_compatible_model_in_memory(
            "custom-provider",
            "custom-model",
            "https://example.test/v1",
            None,
        );
        registry.save().unwrap();

        let removed = registry.remove_custom_model(index).unwrap();

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

        let error = registry.remove_custom_model(index).unwrap_err();

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
