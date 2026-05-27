use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use exgent_core::AppRuntimeHost;

use super::auth_input::{
    handle_add_model_form_key, handle_api_key_input_key, handle_api_key_provider_key,
    handle_auth_method_key, handle_custom_model_kind_key, handle_subscription_provider_key,
};
use super::composer_input::handle_composer_key;
use super::session_input::{
    handle_debug_menu_key, handle_debug_prompt_key, handle_session_picker_key,
};
use super::settings_input::{
    handle_auth_action_key, handle_auth_settings_key, handle_custom_theme_key,
    handle_language_picker_key, handle_model_action_key, handle_model_picker_key,
    handle_model_settings_key, handle_settings_menu_key, handle_theme_picker_key,
    handle_tui_settings_key,
};
use super::state::*;

type TuiRuntime = AppRuntimeHost;

pub(super) fn handle_key(app: &mut TuiApp, runtime: &mut TuiRuntime, key: KeyEvent) -> UiAction {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        if app.quit_is_armed() {
            return UiAction::Quit;
        }
        app.arm_quit();
        app.push_note("press Ctrl+C again to quit");
        return UiAction::None;
    } else {
        app.disarm_quit();
    }

    if transcript_scroll_enabled(app) && handle_transcript_scroll_key(app, key) {
        return UiAction::None;
    }

    match app.overlay.clone() {
        Overlay::ModelPicker(mut picker) => handle_model_picker_key(app, runtime, key, &mut picker),
        Overlay::SettingsMenu(mut state) => handle_settings_menu_key(app, runtime, key, &mut state),
        Overlay::AuthSettings(mut state) => handle_auth_settings_key(app, runtime, key, &mut state),
        Overlay::AuthAction(mut state) => handle_auth_action_key(app, runtime, key, &mut state),
        Overlay::ModelSettings(mut state) => {
            handle_model_settings_key(app, runtime, key, &mut state)
        }
        Overlay::ModelAction(mut state) => handle_model_action_key(app, runtime, key, &mut state),
        Overlay::ThemePicker(mut state) => handle_theme_picker_key(app, runtime, key, &mut state),
        Overlay::TuiSettings(mut state) => handle_tui_settings_key(app, runtime, key, &mut state),
        Overlay::CustomTheme(mut state) => handle_custom_theme_key(app, runtime, key, &mut state),
        Overlay::LanguagePicker(mut state) => {
            handle_language_picker_key(app, runtime, key, &mut state)
        }
        Overlay::SessionPicker(mut state) => {
            handle_session_picker_key(app, runtime, key, &mut state)
        }
        Overlay::DebugMenu(mut state) => handle_debug_menu_key(app, key, &mut state),
        Overlay::DebugPrompt(mut state) => handle_debug_prompt_key(app, runtime, key, &mut state),
        Overlay::AuthMethod(mut state) => handle_auth_method_key(app, runtime, key, &mut state),
        Overlay::ApiKeyProvider(mut state) => {
            handle_api_key_provider_key(app, runtime, key, &mut state)
        }
        Overlay::ApiKeyInput(mut state) => handle_api_key_input_key(app, runtime, key, &mut state),
        Overlay::CustomModelKind(mut state) => {
            handle_custom_model_kind_key(app, runtime, key, &mut state)
        }
        Overlay::AddModelForm(mut state) => {
            handle_add_model_form_key(app, runtime, key, &mut state)
        }
        Overlay::SubscriptionProvider(mut state) => {
            handle_subscription_provider_key(app, key, &mut state)
        }
        Overlay::AuthProgress(_) => UiAction::None,
        Overlay::SlashMenu { selected } => handle_composer_key(app, runtime, key, Some(selected)),
        Overlay::None => handle_composer_key(app, runtime, key, None),
    }
}

fn transcript_scroll_enabled(app: &TuiApp) -> bool {
    matches!(app.overlay, Overlay::None | Overlay::SlashMenu { .. })
}

fn handle_transcript_scroll_key(app: &mut TuiApp, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::PageUp => {
            app.scroll_transcript_up(10);
            true
        }
        KeyCode::PageDown => {
            app.scroll_transcript_down(10);
            true
        }
        KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.scroll_transcript_to_top();
            true
        }
        KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.scroll_transcript_to_bottom();
            true
        }
        _ => false,
    }
}
