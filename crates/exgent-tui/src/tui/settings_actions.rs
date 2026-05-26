use exgent_core::ModelSettingsItem;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AuthProviderSettingsItem {
    pub(super) provider: String,
    pub(super) model_indices: Vec<usize>,
    pub(super) is_enabled: bool,
    pub(super) has_built_in: bool,
}

pub(super) fn enabled_provider_model_settings_items(
    models: &[ModelSettingsItem],
) -> Vec<ModelSettingsItem> {
    models
        .iter()
        .filter(|model| {
            models
                .iter()
                .any(|candidate| candidate.provider == model.provider && candidate.is_enabled)
        })
        .cloned()
        .collect()
}

pub(super) fn auth_provider_settings_items(
    models: &[ModelSettingsItem],
) -> Vec<AuthProviderSettingsItem> {
    let mut providers = Vec::<AuthProviderSettingsItem>::new();
    for model in models {
        if let Some(provider) = providers
            .iter_mut()
            .find(|provider| provider.provider == model.provider)
        {
            provider.model_indices.push(model.index);
            provider.is_enabled |= model.is_enabled;
            provider.has_built_in |= !model.is_custom;
        } else {
            providers.push(AuthProviderSettingsItem {
                provider: model.provider.clone(),
                model_indices: vec![model.index],
                is_enabled: model.is_enabled,
                has_built_in: !model.is_custom,
            });
        }
    }
    providers
}

pub(super) fn enabled_indices_after_provider_action(
    current_models: &[ModelSettingsItem],
    providers: &[AuthProviderSettingsItem],
    target_indices: &[usize],
    enabled: bool,
) -> Vec<usize> {
    let mut enabled_indices = currently_enabled_indices(current_models);

    if enabled {
        for index in target_indices {
            for model_index in &providers[*index].model_indices {
                if !enabled_indices.contains(model_index) {
                    enabled_indices.push(*model_index);
                }
            }
        }
    } else {
        for index in target_indices {
            for model_index in &providers[*index].model_indices {
                enabled_indices.retain(|index| index != model_index);
            }
        }
    }

    enabled_indices
}

pub(super) fn enabled_indices_after_model_action(
    current_models: &[ModelSettingsItem],
    visible_models: &[ModelSettingsItem],
    target_indices: &[usize],
    enabled: bool,
) -> Vec<usize> {
    let mut enabled_indices = currently_enabled_indices(current_models);

    if enabled {
        for index in target_indices {
            let model_index = visible_models[*index].index;
            if !enabled_indices.contains(&model_index) {
                enabled_indices.push(model_index);
            }
        }
    } else {
        for index in target_indices {
            let model_index = visible_models[*index].index;
            enabled_indices.retain(|index| *index != model_index);
        }
    }

    enabled_indices
}

pub(super) fn provider_model_indices_for_removal(
    provider: &AuthProviderSettingsItem,
) -> Vec<usize> {
    if provider.has_built_in {
        provider
            .model_indices
            .first()
            .copied()
            .into_iter()
            .collect()
    } else {
        provider.model_indices.iter().rev().copied().collect()
    }
}

fn currently_enabled_indices(models: &[ModelSettingsItem]) -> Vec<usize> {
    models
        .iter()
        .filter(|model| model.is_enabled)
        .map(|model| model.index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(index: usize, provider: &str, id: &str, is_enabled: bool) -> ModelSettingsItem {
        ModelSettingsItem {
            index,
            provider: provider.to_string(),
            id: id.to_string(),
            name: id.to_string(),
            adapter: "openai-completions".to_string(),
            is_enabled,
            is_custom: false,
        }
    }

    #[test]
    fn filters_models_to_enabled_providers() {
        let models = vec![
            item(0, "one", "enabled", true),
            item(1, "one", "hidden", false),
            item(2, "two", "hidden", false),
        ];

        let visible = enabled_provider_model_settings_items(&models);

        assert_eq!(
            visible
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["enabled", "hidden"]
        );
    }

    #[test]
    fn provider_action_updates_enabled_indices() {
        let models = vec![
            item(0, "one", "a", true),
            item(1, "one", "b", false),
            item(2, "two", "c", true),
        ];
        let providers = auth_provider_settings_items(&models);

        assert_eq!(
            enabled_indices_after_provider_action(&models, &providers, &[0], true),
            vec![0, 2, 1]
        );
        assert_eq!(
            enabled_indices_after_provider_action(&models, &providers, &[0], false),
            vec![2]
        );
    }
}
