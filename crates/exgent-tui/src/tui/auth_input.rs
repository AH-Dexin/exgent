use crossterm::event::{KeyCode, KeyEvent};
use exgent_core::{tr, AppRuntimeHost, CompatibleModelKind, MessageId};

use super::forms::active_add_model_field_mut;
use super::overlays::{open_api_key_provider_overlay, open_subscription_provider_overlay};
use super::state::*;

type TuiRuntime = AppRuntimeHost;

pub(super) fn handle_auth_method_key(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut AuthMethodState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(1);
            app.overlay = Overlay::AuthMethod(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % 2;
            app.overlay = Overlay::AuthMethod(state.clone());
        }
        KeyCode::Enter => {
            if state.selected == 0 {
                open_subscription_provider_overlay(app, runtime);
            } else {
                open_api_key_provider_overlay(app, runtime);
            }
        }
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_api_key_provider_key(
    app: &mut TuiApp,
    _runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut ApiKeyProviderState,
) -> UiAction {
    let item_count = state.providers.len() + 1;
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::AuthMethod(AuthMethodState { selected: 1 }),
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(item_count - 1);
            app.overlay = Overlay::ApiKeyProvider(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % item_count;
            app.overlay = Overlay::ApiKeyProvider(state.clone());
        }
        KeyCode::Enter => {
            if state.selected == state.providers.len() {
                app.overlay = Overlay::CustomModelKind(CustomModelKindState { selected: 0 });
            } else if let Some(provider) = state.providers.get(state.selected) {
                app.overlay = Overlay::ApiKeyInput(ApiKeyInputState {
                    provider: provider.provider.clone(),
                    value: String::new(),
                });
            }
        }
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_custom_model_kind_key(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut CustomModelKindState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => open_api_key_provider_overlay(app, runtime),
        KeyCode::Up => {
            state.selected = state
                .selected
                .checked_sub(1)
                .unwrap_or(CompatibleModelKind::ALL.len() - 1);
            app.overlay = Overlay::CustomModelKind(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % CompatibleModelKind::ALL.len();
            app.overlay = Overlay::CustomModelKind(state.clone());
        }
        KeyCode::Enter => {
            let kind =
                CompatibleModelKind::ALL[state.selected.min(CompatibleModelKind::ALL.len() - 1)];
            app.overlay = Overlay::AddModelForm(AddModelFormState::for_kind(kind));
        }
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_api_key_input_key(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut ApiKeyInputState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => open_api_key_provider_overlay(app, runtime),
        KeyCode::Backspace => {
            state.value.pop();
            app.overlay = Overlay::ApiKeyInput(state.clone());
        }
        KeyCode::Enter => {
            let token = state.value.trim();
            let result = if token.is_empty() {
                runtime
                    .remove_auth_token(&state.provider)
                    .map(|_| format!("removed auth token for {}", state.provider))
            } else {
                runtime
                    .set_auth_token(&state.provider, token)
                    .map(|_| format!("saved auth token for {}", state.provider))
            };
            match result {
                Ok(message) => app.push_note(message),
                Err(error) => app.push_error(error),
            }
            app.refresh_status(runtime);
            open_api_key_provider_overlay(app, runtime);
        }
        KeyCode::Char(value) => {
            state.value.push(value);
            app.overlay = Overlay::ApiKeyInput(state.clone());
        }
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_add_model_form_key(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut AddModelFormState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => open_api_key_provider_overlay(app, runtime),
        KeyCode::Backspace => {
            active_add_model_field_mut(state).pop();
            app.overlay = Overlay::AddModelForm(state.clone());
        }
        KeyCode::Tab | KeyCode::Down => {
            state.field = (state.field + 1) % 4;
            app.overlay = Overlay::AddModelForm(state.clone());
        }
        KeyCode::Up => {
            state.field = state.field.checked_sub(1).unwrap_or(3);
            app.overlay = Overlay::AddModelForm(state.clone());
        }
        KeyCode::Enter => {
            if state.field < 3 {
                state.field += 1;
                app.overlay = Overlay::AddModelForm(state.clone());
                return UiAction::None;
            }
            if state.provider.trim().is_empty()
                || state.model_id.trim().is_empty()
                || state.base_url.trim().is_empty()
            {
                app.push_error("provider, model id, and base url are required");
                app.overlay = Overlay::AddModelForm(state.clone());
                return UiAction::None;
            }
            match runtime.add_compatible_model(
                state.kind,
                state.provider.trim(),
                state.model_id.trim(),
                state.base_url.trim(),
                state.api_key.trim(),
            ) {
                Ok(info) => {
                    app.push_note(format!(
                        "added model {} / {} at index {}",
                        info.provider,
                        info.model_id,
                        info.index + 1
                    ));
                    app.refresh_status(runtime);
                    open_api_key_provider_overlay(app, runtime);
                }
                Err(error) => {
                    app.push_error(error);
                    app.overlay = Overlay::AddModelForm(state.clone());
                }
            }
        }
        KeyCode::Char(value) => {
            active_add_model_field_mut(state).push(value);
            app.overlay = Overlay::AddModelForm(state.clone());
        }
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_subscription_provider_key(
    app: &mut TuiApp,
    key: KeyEvent,
    state: &mut SubscriptionProviderState,
) -> UiAction {
    if state.providers.is_empty() {
        app.overlay = Overlay::None;
        app.push_note(tr(app.locale, MessageId::NoSubscriptionProviders));
        return UiAction::None;
    }
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up => {
            state.selected = state
                .selected
                .checked_sub(1)
                .unwrap_or(state.providers.len() - 1);
            app.overlay = Overlay::SubscriptionProvider(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % state.providers.len();
            app.overlay = Overlay::SubscriptionProvider(state.clone());
        }
        KeyCode::Enter => {
            let provider = state.providers[state.selected].clone();
            app.overlay = Overlay::None;
            return UiAction::RunSubscriptionAuth(provider);
        }
        _ => {}
    }
    UiAction::None
}
