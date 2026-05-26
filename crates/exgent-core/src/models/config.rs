use std::collections::BTreeMap;

use crate::ai::{Model, ModelCost};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct ModelsConfig {
    pub models: Vec<ConfigModel>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct ConfigModel {
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
    pub(super) fn compatible(
        provider: impl Into<String>,
        id: impl Into<String>,
        adapter: impl Into<String>,
        base_url: impl Into<String>,
        api_key_env: Option<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            id: id.into(),
            name: None,
            adapter: adapter.into(),
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
    }

    pub(super) fn from_model(model: &Model) -> Self {
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

    pub(super) fn into_model(self) -> Model {
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
