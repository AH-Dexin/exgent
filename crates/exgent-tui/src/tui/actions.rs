use exgent_core::{AppRuntimeHost, ModelSettingsItem};

use super::settings_actions::{
    enabled_indices_after_model_action, enabled_indices_after_provider_action,
    provider_model_indices_for_removal, AuthProviderSettingsItem as AuthProviderItem,
};
use super::state::{TranscriptItem, TuiApp};

type TuiRuntime = AppRuntimeHost;

pub(super) fn set_providers_enabled(
    runtime: &mut TuiRuntime,
    providers: &[AuthProviderItem],
    targets: &[usize],
    enabled: bool,
) -> Result<String, String> {
    let current_models = runtime.model_settings_items();
    let enabled_indices =
        enabled_indices_after_provider_action(&current_models, providers, targets, enabled);

    runtime.set_enabled_model_indices(&enabled_indices)?;
    Ok(format!(
        "{} provider(s) {}",
        targets.len(),
        if enabled { "enabled" } else { "disabled" }
    ))
}

pub(super) fn remove_providers(
    runtime: &mut TuiRuntime,
    providers: &[AuthProviderItem],
    targets: &[usize],
) -> Result<String, String> {
    let mut messages = Vec::new();
    for target in targets.iter().rev() {
        let provider = &providers[*target];
        for index in provider_model_indices_for_removal(provider) {
            messages.push(runtime.delete_model(index)?);
        }
    }
    if messages.is_empty() {
        Ok("no providers removed".to_string())
    } else {
        Ok(messages.join("\n"))
    }
}

pub(super) fn set_model_items_enabled(
    runtime: &mut TuiRuntime,
    models: &[ModelSettingsItem],
    targets: &[usize],
    enabled: bool,
) -> Result<String, String> {
    let current_models = runtime.model_settings_items();
    let enabled_indices =
        enabled_indices_after_model_action(&current_models, models, targets, enabled);

    runtime.set_enabled_model_indices(&enabled_indices)?;
    Ok(format!(
        "{} model(s) {}",
        targets.len(),
        if enabled { "enabled" } else { "disabled" }
    ))
}

pub(super) fn load_recent_messages(app: &mut TuiApp, runtime: &TuiRuntime) {
    for message in runtime.recent_messages(20) {
        match message.role.as_str() {
            "user" => app.transcript.push(TranscriptItem::User(message.content)),
            "assistant" => app
                .transcript
                .push(TranscriptItem::Assistant(message.content)),
            _ => app.transcript.push(TranscriptItem::Note(message.content)),
        }
    }
}
