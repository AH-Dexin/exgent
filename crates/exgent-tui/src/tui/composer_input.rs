use crossterm::event::{KeyCode, KeyEvent};
use exgent_core::{tr, AppRuntimeHost, MessageId};

use crate::commands::{parse_command, AppCommand};

use super::forms::{active_add_model_field_mut, paste_theme_value};
use super::overlays::{
    open_auth_settings_overlay, open_language_picker_overlay, open_model_picker_overlay,
    open_model_settings_overlay, open_session_picker_overlay, open_theme_picker_overlay,
};
use super::state::*;
use super::suggestions::slash_suggestions;

type TuiRuntime = AppRuntimeHost;

pub(super) fn handle_composer_key(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    key: KeyEvent,
    slash_selected: Option<usize>,
) -> UiAction {
    if app.is_running {
        return UiAction::None;
    }

    match key.code {
        KeyCode::Esc => {
            app.overlay = Overlay::None;
        }
        KeyCode::Backspace => {
            app.composer.input.pop();
            app.composer.history_index = None;
            app.sync_slash_menu();
        }
        KeyCode::Tab => {
            if let Some(command) = slash_suggestions(&app.composer.input).first() {
                app.composer.input = command.command.to_string();
                app.sync_slash_menu();
            }
        }
        KeyCode::Up if app.composer.input.starts_with('/') => {
            move_slash_selection(app, slash_selected, -1);
        }
        KeyCode::Down if app.composer.input.starts_with('/') => {
            move_slash_selection(app, slash_selected, 1);
        }
        KeyCode::Up => recall_history(app, -1),
        KeyCode::Down => recall_history(app, 1),
        KeyCode::Enter => return submit_input(app, runtime, slash_selected),
        KeyCode::Char(value) => {
            app.composer.input.push(value);
            app.composer.history_index = None;
            app.sync_slash_menu();
        }
        _ => {}
    }
    UiAction::None
}

fn move_slash_selection(app: &mut TuiApp, selected: Option<usize>, delta: isize) {
    let suggestions = slash_suggestions(&app.composer.input);
    if suggestions.is_empty() {
        return;
    }
    let selected = selected.unwrap_or(0);
    let next = if delta < 0 {
        selected.checked_sub(1).unwrap_or(suggestions.len() - 1)
    } else {
        (selected + 1) % suggestions.len()
    };
    app.overlay = Overlay::SlashMenu { selected: next };
}

fn recall_history(app: &mut TuiApp, delta: isize) {
    if app.composer.history.is_empty() {
        return;
    }
    let len = app.composer.history.len();
    let next = match (app.composer.history_index, delta < 0) {
        (Some(index), true) => index.saturating_sub(1),
        (Some(index), false) if index + 1 < len => index + 1,
        (Some(_), false) => {
            app.composer.history_index = None;
            app.composer.input = app.composer.draft.clone();
            return;
        }
        (None, true) => {
            app.composer.draft = app.composer.input.clone();
            len - 1
        }
        (None, false) => return,
    };
    app.composer.history_index = Some(next);
    app.composer.input = app.composer.history[next].clone();
    app.sync_slash_menu();
}

fn submit_input(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    slash_selected: Option<usize>,
) -> UiAction {
    let mut input = app.composer.input.trim().to_string();
    if input.is_empty() {
        return UiAction::None;
    }

    if input.starts_with('/')
        && parse_command(&input).is_some_and(|command| matches!(command, AppCommand::Unknown(_)))
    {
        if let Some(index) = slash_selected {
            if let Some(command) = slash_suggestions(&input).get(index) {
                input = command.command.to_string();
            }
        }
    }

    if app.composer.history.last().map(|entry| entry.as_str()) != Some(input.as_str()) {
        app.composer.history.push(input.clone());
    }
    app.composer.history_index = None;
    app.composer.input.clear();
    app.overlay = Overlay::None;

    match parse_command(&input) {
        Some(AppCommand::Quit) => UiAction::Quit,
        Some(AppCommand::Model) => {
            open_model_picker_overlay(app, runtime);
            UiAction::None
        }
        Some(AppCommand::Settings) => {
            app.overlay = Overlay::SettingsMenu(SettingsMenuState { selected: 0 });
            UiAction::None
        }
        Some(AppCommand::SettingsAuth) => {
            open_auth_settings_overlay(app, runtime);
            UiAction::None
        }
        Some(AppCommand::SettingsModel) => {
            open_model_settings_overlay(app, runtime);
            UiAction::None
        }
        Some(AppCommand::SettingsTheme) => {
            open_theme_picker_overlay(app, runtime);
            UiAction::None
        }
        Some(AppCommand::SettingsLanguage) => {
            open_language_picker_overlay(app, runtime);
            UiAction::None
        }
        Some(AppCommand::Session) => {
            open_session_picker_overlay(app, runtime);
            UiAction::None
        }
        Some(AppCommand::Compact) => {
            match runtime.compact_context() {
                Ok(count) => app.push_note(
                    tr(app.locale, MessageId::CompactSuccess)
                        .replace("{count}", &count.to_string()),
                ),
                Err(error) => app.push_error(error),
            }
            UiAction::None
        }
        Some(AppCommand::DebugEnable) => {
            match runtime.set_prompt_display_enabled(true) {
                Ok(()) => app.push_note(tr(app.locale, MessageId::DebugDisplayEnabled)),
                Err(error) => app.push_error(error),
            }
            app.refresh_status(runtime);
            UiAction::None
        }
        Some(AppCommand::DebugDisable) => {
            match runtime.set_prompt_display_enabled(false) {
                Ok(()) => app.push_note(tr(app.locale, MessageId::DebugDisplayDisabled)),
                Err(error) => app.push_error(error),
            }
            app.refresh_status(runtime);
            UiAction::None
        }
        Some(AppCommand::DebugShow) => {
            app.transcript
                .push(TranscriptItem::Note(runtime.system_prompt()));
            UiAction::None
        }
        Some(AppCommand::Auth) => {
            app.overlay = Overlay::AuthMethod(AuthMethodState { selected: 0 });
            UiAction::None
        }
        Some(AppCommand::Debug) => {
            app.overlay = Overlay::DebugMenu(DebugMenuState { selected: 0 });
            UiAction::None
        }
        Some(AppCommand::Unknown(command)) => {
            app.push_error(format!(
                "{}: {command}",
                tr(app.locale, MessageId::UnknownCommand)
            ));
            UiAction::None
        }
        None => UiAction::RunPrompt(input),
    }
}

pub(super) fn handle_paste(app: &mut TuiApp, value: &str) {
    if app.is_running {
        return;
    }

    match &mut app.overlay {
        Overlay::ApiKeyInput(state) => {
            state.value.push_str(value);
        }
        Overlay::AddModelForm(state) => {
            active_add_model_field_mut(state).push_str(value);
        }
        Overlay::CustomTheme(state) => {
            paste_theme_value(state, value);
        }
        Overlay::None | Overlay::SlashMenu { .. } => {
            app.composer.input.push_str(value);
            app.composer.history_index = None;
            app.sync_slash_menu();
        }
        _ => {}
    }
}
