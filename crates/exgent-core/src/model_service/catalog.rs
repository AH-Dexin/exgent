use std::env;

use exgent_ai::{Model, SubscriptionProvider};

use crate::{
    auth::AuthStore,
    models::{ModelAvailabilityCache, ModelRegistry},
    settings::{ModelSelection, SettingsStore},
};

pub(super) fn model_with_auth(model: &Model, auth: &AuthStore) -> Model {
    let mut model = model.clone();
    if let Some(token) = auth.token(&model.provider) {
        model.api_key = Some(token.to_string());
    }
    model
}

pub(super) fn resolve_initial_model_index(
    models: &ModelRegistry,
    model_cache: &ModelAvailabilityCache,
    default_model: Option<&ModelSelection>,
    settings: &SettingsStore,
    auth: &AuthStore,
    subscription_providers: &[SubscriptionProvider],
) -> Option<usize> {
    if let Some(index) =
        default_model.and_then(|selection| models.model_index_for_selection(selection))
    {
        if models
            .get(index)
            .map(|model| {
                is_selectable_model(models, model_cache, settings, auth, index, model)
                    || (model.provider == "fake" && is_configured_model(model, auth))
            })
            .unwrap_or(false)
        {
            return Some(index);
        }
    }

    subscription_providers
        .iter()
        .find_map(|spec| {
            auth.has_oauth(&spec.id).then(|| {
                models
                    .models()
                    .iter()
                    .enumerate()
                    .find_map(|(index, model)| {
                        (model.provider == spec.id
                            && is_available_model(models, model_cache, index, model)
                            && is_configured_model(model, auth)
                            && settings.is_model_enabled(model))
                        .then_some(index)
                    })
            })?
        })
        .or_else(|| first_selectable_model_index(models, model_cache, settings, auth))
}

pub(super) fn first_selectable_model_index(
    models: &ModelRegistry,
    model_cache: &ModelAvailabilityCache,
    settings: &SettingsStore,
    auth: &AuthStore,
) -> Option<usize> {
    models
        .models()
        .iter()
        .enumerate()
        .find_map(|(index, model)| {
            is_selectable_model(models, model_cache, settings, auth, index, model).then_some(index)
        })
}

pub(super) fn is_selectable_model(
    models: &ModelRegistry,
    model_cache: &ModelAvailabilityCache,
    settings: &SettingsStore,
    auth: &AuthStore,
    index: usize,
    model: &Model,
) -> bool {
    model.provider != "fake"
        && is_available_model(models, model_cache, index, model)
        && is_configured_model(model, auth)
        && settings.is_model_enabled(model)
}

pub(super) fn is_available_model(
    models: &ModelRegistry,
    model_cache: &ModelAvailabilityCache,
    index: usize,
    model: &Model,
) -> bool {
    models.is_custom_model(index) || model_cache.allows_model(model)
}

pub(super) fn is_configured_model(model: &Model, auth: &AuthStore) -> bool {
    model.adapter == "fake"
        || auth.has_api_key(&model.provider)
        || auth.has_oauth(&model.provider)
        || model.api_key.as_deref().map(is_non_empty).unwrap_or(false)
        || model
            .api_key_env
            .as_deref()
            .and_then(|name| env::var(name).ok())
            .as_deref()
            .map(is_non_empty)
            .unwrap_or(false)
}

fn is_non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

pub fn no_model_configured_message() -> &'static str {
    "No model configured. Please enter /auth to configure a model."
}
