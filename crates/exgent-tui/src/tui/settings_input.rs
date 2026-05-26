use crossterm::event::{KeyCode, KeyEvent};
use exgent_core::{tr, AppRuntimeHost, MessageId, ThemeSettings, LANGUAGE_OPTIONS, THEME_PRESETS};

use super::actions::{remove_providers, set_model_items_enabled, set_providers_enabled};
use super::forms::{
    active_theme_field_mut, checked_or_focused_indices, parse_custom_theme, preview_theme_selection,
};
use super::overlays::{
    open_auth_settings_overlay, open_language_picker_overlay, open_model_settings_overlay,
    open_theme_picker_overlay,
};
use super::state::*;

type TuiRuntime = AppRuntimeHost;

pub(super) fn handle_model_picker_key(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    key: KeyEvent,
    picker: &mut ModelPickerState,
) -> UiAction {
    if picker.models.is_empty() {
        app.overlay = Overlay::None;
        app.push_note(tr(app.locale, MessageId::NoModelsAvailable));
        return UiAction::None;
    }

    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up => {
            picker.selected = if picker.selected == 0 {
                picker.models.len() - 1
            } else {
                picker.selected - 1
            };
            app.overlay = Overlay::ModelPicker(picker.clone());
        }
        KeyCode::Down => {
            picker.selected = (picker.selected + 1) % picker.models.len();
            app.overlay = Overlay::ModelPicker(picker.clone());
        }
        KeyCode::Enter => {
            let model = &picker.models[picker.selected];
            match runtime.select_model(model.index) {
                Ok(()) => {
                    app.refresh_status(runtime);
                    app.push_note(format!("selected model: {}", runtime.model_label()));
                }
                Err(error) => app.push_error(error),
            }
            app.overlay = Overlay::None;
        }
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_settings_menu_key(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut SettingsMenuState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(3);
            app.overlay = Overlay::SettingsMenu(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % 4;
            app.overlay = Overlay::SettingsMenu(state.clone());
        }
        KeyCode::Enter => match state.selected {
            0 => open_auth_settings_overlay(app, runtime),
            1 => open_model_settings_overlay(app, runtime),
            2 => open_theme_picker_overlay(app, runtime),
            _ => open_language_picker_overlay(app, runtime),
        },
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_auth_settings_key(
    app: &mut TuiApp,
    _runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut AuthSettingsState,
) -> UiAction {
    if state.providers.is_empty() {
        app.overlay = Overlay::None;
        app.push_note(tr(app.locale, MessageId::NoConfiguredProviders));
        return UiAction::None;
    }

    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up => {
            state.selected = state
                .selected
                .checked_sub(1)
                .unwrap_or(state.providers.len() - 1);
            app.overlay = Overlay::AuthSettings(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % state.providers.len();
            app.overlay = Overlay::AuthSettings(state.clone());
        }
        KeyCode::Char(' ') => {
            if let Some(checked) = state.checked.get_mut(state.selected) {
                *checked = !*checked;
            }
            app.overlay = Overlay::AuthSettings(state.clone());
        }
        KeyCode::Enter => {
            let targets = checked_or_focused_indices(&state.checked, state.selected);
            app.overlay = Overlay::AuthAction(AuthActionState {
                providers: state.providers.clone(),
                targets,
                selected: 0,
            });
        }
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_auth_action_key(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut AuthActionState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => {
            app.overlay = Overlay::AuthSettings(AuthSettingsState {
                providers: state.providers.clone(),
                selected: state.targets.first().copied().unwrap_or(0),
                checked: vec![false; state.providers.len()],
            });
        }
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(2);
            app.overlay = Overlay::AuthAction(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % 3;
            app.overlay = Overlay::AuthAction(state.clone());
        }
        KeyCode::Enter => {
            let message = match state.selected {
                0 => set_providers_enabled(runtime, &state.providers, &state.targets, true),
                1 => set_providers_enabled(runtime, &state.providers, &state.targets, false),
                2 => remove_providers(runtime, &state.providers, &state.targets),
                _ => Ok(String::new()),
            };
            match message {
                Ok(message) if !message.is_empty() => app.push_note(message),
                Ok(_) => {}
                Err(error) => app.push_error(error),
            }
            app.refresh_status(runtime);
            open_auth_settings_overlay(app, runtime);
        }
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_model_settings_key(
    app: &mut TuiApp,
    _runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut ModelSettingsState,
) -> UiAction {
    if state.models.is_empty() {
        app.overlay = Overlay::None;
        app.push_note(tr(app.locale, MessageId::NoConfiguredModelsEnableProvider));
        return UiAction::None;
    }

    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up => {
            state.selected = state
                .selected
                .checked_sub(1)
                .unwrap_or(state.models.len() - 1);
            app.overlay = Overlay::ModelSettings(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % state.models.len();
            app.overlay = Overlay::ModelSettings(state.clone());
        }
        KeyCode::Char(' ') => {
            if let Some(checked) = state.checked.get_mut(state.selected) {
                *checked = !*checked;
            }
            app.overlay = Overlay::ModelSettings(state.clone());
        }
        KeyCode::Enter => {
            let targets = checked_or_focused_indices(&state.checked, state.selected);
            app.overlay = Overlay::ModelAction(ModelActionState {
                models: state.models.clone(),
                targets,
                selected: 0,
            });
        }
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_model_action_key(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut ModelActionState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => {
            app.overlay = Overlay::ModelSettings(ModelSettingsState {
                models: state.models.clone(),
                selected: state.targets.first().copied().unwrap_or(0),
                checked: vec![false; state.models.len()],
            });
        }
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(1);
            app.overlay = Overlay::ModelAction(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % 2;
            app.overlay = Overlay::ModelAction(state.clone());
        }
        KeyCode::Enter => {
            let message = set_model_items_enabled(
                runtime,
                &state.models,
                &state.targets,
                state.selected == 0,
            );
            match message {
                Ok(message) => app.push_note(message),
                Err(error) => app.push_error(error),
            }
            app.refresh_status(runtime);
            open_model_settings_overlay(app, runtime);
        }
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_theme_picker_key(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut ThemePickerState,
) -> UiAction {
    let item_count = THEME_PRESETS.len() + 1;
    match key.code {
        KeyCode::Esc => {
            app.theme_preview = None;
            app.overlay = Overlay::SettingsMenu(SettingsMenuState { selected: 2 });
        }
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(item_count - 1);
            preview_theme_selection(app, runtime, state.selected);
            app.overlay = Overlay::ThemePicker(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % item_count;
            preview_theme_selection(app, runtime, state.selected);
            app.overlay = Overlay::ThemePicker(state.clone());
        }
        KeyCode::Enter => {
            if let Some(preset) = THEME_PRESETS.get(state.selected).copied() {
                let theme = ThemeSettings::preset(preset);
                match runtime.set_theme(theme.clone()) {
                    Ok(()) => {
                        app.theme_preview = None;
                        app.refresh_status(runtime);
                        app.push_note(
                            tr(app.locale, MessageId::ThemeSaved).replace("{name}", &theme.name),
                        );
                    }
                    Err(error) => app.push_error(error),
                }
                app.overlay = Overlay::None;
            } else {
                app.theme_preview = None;
                app.overlay = Overlay::CustomTheme(CustomThemeState::default());
            }
        }
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_custom_theme_key(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut CustomThemeState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => open_theme_picker_overlay(app, runtime),
        KeyCode::Backspace => {
            active_theme_field_mut(state).pop();
            app.overlay = Overlay::CustomTheme(state.clone());
        }
        KeyCode::Tab | KeyCode::Down => {
            state.field = (state.field + 1) % 3;
            app.overlay = Overlay::CustomTheme(state.clone());
        }
        KeyCode::Up => {
            state.field = state.field.checked_sub(1).unwrap_or(2);
            app.overlay = Overlay::CustomTheme(state.clone());
        }
        KeyCode::Enter => {
            if state.field < 2 {
                state.field += 1;
                app.overlay = Overlay::CustomTheme(state.clone());
                return UiAction::None;
            }
            match parse_custom_theme(state) {
                Ok(theme) => match runtime.set_theme(theme.clone()) {
                    Ok(()) => {
                        app.theme_preview = None;
                        app.refresh_status(runtime);
                        app.push_note(tr(app.locale, MessageId::ThemeSaved).replace(
                            "{name}",
                            &format!(
                                "custom rgb({}, {}, {})",
                                theme.rgb.r, theme.rgb.g, theme.rgb.b
                            ),
                        ));
                        app.overlay = Overlay::None;
                    }
                    Err(error) => {
                        app.push_error(error);
                        app.overlay = Overlay::CustomTheme(state.clone());
                    }
                },
                Err(error) => {
                    app.push_error(error);
                    app.overlay = Overlay::CustomTheme(state.clone());
                }
            }
        }
        KeyCode::Char(value) if value.is_ascii_digit() => {
            let field = active_theme_field_mut(state);
            if field.len() < 3 {
                field.push(value);
            }
            app.overlay = Overlay::CustomTheme(state.clone());
        }
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_language_picker_key(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut LanguagePickerState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => {
            app.overlay = Overlay::SettingsMenu(SettingsMenuState { selected: 3 });
        }
        KeyCode::Up => {
            state.selected = state
                .selected
                .checked_sub(1)
                .unwrap_or(LANGUAGE_OPTIONS.len().saturating_sub(1));
            app.overlay = Overlay::LanguagePicker(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % LANGUAGE_OPTIONS.len();
            app.overlay = Overlay::LanguagePicker(state.clone());
        }
        KeyCode::Enter => {
            let option = LANGUAGE_OPTIONS[state.selected.min(LANGUAGE_OPTIONS.len() - 1)];
            match runtime.set_locale(option.setting) {
                Ok(()) => {
                    app.refresh_status(runtime);
                    app.push_note(
                        tr(app.locale, MessageId::LanguageSaved)
                            .replace("{name}", option.locale.display_name()),
                    );
                    app.overlay = Overlay::None;
                }
                Err(error) => app.push_error(error),
            }
        }
        _ => {}
    }
    UiAction::None
}
