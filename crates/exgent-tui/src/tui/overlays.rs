use exgent_core::{tr, AppRuntimeHost, MessageId, LANGUAGE_OPTIONS, THEME_PRESETS};

use super::forms::preview_theme_selection;
use super::settings_actions::{
    auth_provider_settings_items as auth_provider_items, enabled_provider_model_settings_items,
};
use super::state::{
    ApiKeyProviderState, AuthSettingsState, LanguagePickerState, ModelPickerState,
    ModelSettingsState, Overlay, SessionPickerState, SubscriptionProviderState, ThemePickerState,
    TuiApp,
};

type TuiRuntime = AppRuntimeHost;

pub(super) fn open_model_picker_overlay(app: &mut TuiApp, runtime: &TuiRuntime) {
    let models = runtime.selectable_models();
    if models.is_empty() {
        app.push_note(tr(app.locale, MessageId::NoEnabledModelsAvailable));
    } else {
        let selected = models
            .iter()
            .position(|model| model.is_current)
            .unwrap_or(0);
        app.overlay = Overlay::ModelPicker(ModelPickerState { models, selected });
    }
}

pub(super) fn open_auth_settings_overlay(app: &mut TuiApp, runtime: &TuiRuntime) {
    let providers = auth_provider_items(&runtime.model_settings_items());
    if providers.is_empty() {
        app.overlay = Overlay::None;
        app.push_note(tr(app.locale, MessageId::NoConfiguredModelsAuthFirst));
        return;
    }
    app.overlay = Overlay::AuthSettings(AuthSettingsState {
        checked: vec![false; providers.len()],
        providers,
        selected: 0,
    });
}

pub(super) fn open_model_settings_overlay(app: &mut TuiApp, runtime: &TuiRuntime) {
    let models = enabled_provider_model_settings_items(&runtime.model_settings_items());
    if models.is_empty() {
        app.overlay = Overlay::None;
        app.push_note(tr(app.locale, MessageId::NoConfiguredModelsEnableProvider));
        return;
    }
    app.overlay = Overlay::ModelSettings(ModelSettingsState {
        checked: vec![false; models.len()],
        models,
        selected: 0,
    });
}

pub(super) fn open_theme_picker_overlay(app: &mut TuiApp, runtime: &TuiRuntime) {
    let theme = runtime.theme();
    let selected = THEME_PRESETS
        .iter()
        .position(|preset| preset.name == theme.name && preset.rgb == theme.rgb)
        .unwrap_or(THEME_PRESETS.len());
    app.overlay = Overlay::ThemePicker(ThemePickerState { selected });
    preview_theme_selection(app, runtime, selected);
}

pub(super) fn open_language_picker_overlay(app: &mut TuiApp, runtime: &TuiRuntime) {
    let locale = runtime.locale();
    let selected = LANGUAGE_OPTIONS
        .iter()
        .position(|option| option.locale == locale)
        .unwrap_or(0);
    app.overlay = Overlay::LanguagePicker(LanguagePickerState { selected });
}

pub(super) fn open_session_picker_overlay(app: &mut TuiApp, runtime: &TuiRuntime) {
    match runtime.list_sessions() {
        Ok(mut sessions) => {
            let current = runtime.session_id().to_string();
            sessions.retain(|session| session.id != current);
            app.overlay = Overlay::SessionPicker(SessionPickerState {
                sessions,
                selected: 0,
            });
        }
        Err(error) => app.push_error(error),
    }
}

pub(super) fn open_subscription_provider_overlay(app: &mut TuiApp, runtime: &TuiRuntime) {
    let providers = runtime.subscription_providers();
    if providers.is_empty() {
        app.overlay = Overlay::None;
        app.push_note(tr(app.locale, MessageId::NoSubscriptionProviders));
        return;
    }
    app.overlay = Overlay::SubscriptionProvider(SubscriptionProviderState {
        providers,
        selected: 0,
    });
}

pub(super) fn open_api_key_provider_overlay(app: &mut TuiApp, runtime: &TuiRuntime) {
    app.overlay = Overlay::ApiKeyProvider(ApiKeyProviderState {
        providers: runtime.auth_providers(),
        selected: 0,
    });
}
