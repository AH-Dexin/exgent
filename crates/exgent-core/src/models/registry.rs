use std::{fs, io, path::PathBuf};

use exgent_ai::{built_in_models, DiscoveredModel, Model};

use crate::{config::config_file, persistence::write_json_pretty, settings::ModelSelection};

use super::{
    cache::ModelAvailabilityCache,
    config::{ConfigModel, ModelsConfig},
    CompatibleModelKind,
};

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

    pub fn add_compatible_model_in_memory(
        &mut self,
        kind: CompatibleModelKind,
        provider: impl Into<String>,
        model_id: impl Into<String>,
        base_url: impl Into<String>,
        api_key_env: Option<String>,
    ) -> usize {
        let model =
            ConfigModel::compatible(provider, model_id, kind.adapter(), base_url, api_key_env)
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

    pub fn remove_custom_model_in_memory(&mut self, index: usize) -> io::Result<Model> {
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
        Ok(model)
    }

    pub fn apply_availability_cache(&mut self, cache: &ModelAvailabilityCache) {
        for (provider, models) in cache.providers() {
            for model in models {
                self.merge_discovered_model(provider, model);
            }
        }
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

    fn merge_discovered_model(&mut self, provider: &str, discovered: &DiscoveredModel) {
        if let Some(existing) = self
            .models
            .iter_mut()
            .find(|model| model.provider == provider && model.id == discovered.id)
        {
            if existing.name.is_none() {
                existing.name = discovered.name.clone();
            }
            return;
        }

        let Some(template) = self
            .models
            .iter()
            .find(|model| model.provider == provider)
            .cloned()
        else {
            return;
        };

        let mut model = Model::new(provider, discovered.id.clone(), template.adapter);
        model.name = discovered.name.clone();
        model.base_url = template.base_url;
        model.api_key_env = template.api_key_env;
        model.headers = template.headers;
        model.compat = template.compat;

        self.models.push(model);
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

fn models_config_path(config_path: Option<&str>) -> PathBuf {
    config_file(config_path, "models.json")
}

fn invalid_data(error: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}
