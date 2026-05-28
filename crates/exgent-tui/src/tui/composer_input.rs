use crossterm::event::{KeyCode, KeyEvent};
use exgent_core::{tr, AppRuntimeHost, MessageId};

use crate::commands::{parse_command, AppCommand};

use super::clipboard_image::read_clipboard_image;
use super::clipboard_text::read_clipboard_text;
use super::forms::{active_add_model_field_mut, paste_theme_value};
use super::key_shortcuts::is_paste_shortcut;
use super::overlays::{
    open_auth_settings_overlay, open_language_picker_overlay, open_model_picker_overlay,
    open_model_settings_overlay, open_session_picker_overlay, open_theme_picker_overlay,
    open_tui_settings_overlay,
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
    if is_paste_shortcut(&key) {
        return paste_clipboard(app);
    }

    match key.code {
        KeyCode::Esc => {
            if !app.composer.is_empty() {
                app.composer.clear();
                app.composer.draft.clear();
                app.composer.history_index = None;
                app.sync_slash_menu();
            } else {
                app.overlay = Overlay::None;
            }
        }
        KeyCode::Backspace if app.composer.backspace() => {
            app.composer.history_index = None;
            app.sync_slash_menu();
        }
        KeyCode::Tab if app.composer.images.is_empty() && !app.composer.is_multiline() => {
            if let Some(command) = slash_suggestions(&app.composer.input).first() {
                app.composer.set_text(command.command);
                app.sync_slash_menu();
            }
        }
        KeyCode::Up
            if app.composer.images.is_empty()
                && !app.composer.is_multiline()
                && app.composer.input.starts_with('/') =>
        {
            move_slash_selection(app, slash_selected, -1);
        }
        KeyCode::Down
            if app.composer.images.is_empty()
                && !app.composer.is_multiline()
                && app.composer.input.starts_with('/') =>
        {
            move_slash_selection(app, slash_selected, 1);
        }
        KeyCode::Up if app.composer.is_multiline() => app.composer.move_cursor_up(),
        KeyCode::Down if app.composer.is_multiline() => app.composer.move_cursor_down(),
        KeyCode::Up => recall_history(app, -1),
        KeyCode::Down => recall_history(app, 1),
        KeyCode::Left => app.composer.move_cursor_left(),
        KeyCode::Right => app.composer.move_cursor_right(),
        KeyCode::Home => app.composer.move_cursor_to_start(),
        KeyCode::End => app.composer.move_cursor_to_end(),
        KeyCode::Enter if app.is_running => {}
        KeyCode::Enter => return submit_input(app, runtime, slash_selected),
        KeyCode::Char(value) => {
            app.composer.insert_char(value);
            app.composer.history_index = None;
            app.sync_slash_menu();
        }
        _ => {}
    }
    UiAction::None
}

fn paste_clipboard(app: &mut TuiApp) -> UiAction {
    match read_clipboard_image() {
        Ok(Some(image)) => {
            app.composer.insert_image(image);
            app.composer.history_index = None;
            app.sync_slash_menu();
            return UiAction::None;
        }
        Ok(None) => {}
        Err(error) => app.push_error(error),
    }

    match read_clipboard_text() {
        Ok(Some(text)) if !text.is_empty() => {
            app.insert_paste_text(&text);
        }
        Ok(_) => {}
        Err(error) => app.push_error(error),
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
    if app.composer.history.is_empty() || !app.composer.images.is_empty() {
        return;
    }
    let len = app.composer.history.len();
    let next = match (app.composer.history_index, delta < 0) {
        (Some(index), true) => index.saturating_sub(1),
        (Some(index), false) if index + 1 < len => index + 1,
        (Some(_), false) => {
            app.composer.history_index = None;
            let draft = app.composer.draft.clone();
            app.composer.set_text(draft);
            return;
        }
        (None, true) => {
            app.composer.draft = app.composer.input.clone();
            len - 1
        }
        (None, false) => return,
    };
    app.composer.history_index = Some(next);
    let recalled = app.composer.history[next].clone();
    app.composer.set_text(recalled);
    app.sync_slash_menu();
}

fn submit_input(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    slash_selected: Option<usize>,
) -> UiAction {
    let mut input = app.composer.input.trim().to_string();
    let has_images = !app.composer.images.is_empty();
    let is_multiline = app.composer.is_multiline();
    if input.is_empty() && !has_images {
        return UiAction::None;
    }

    let is_command_input = !has_images && !is_multiline;
    if is_command_input
        && input.starts_with('/')
        && parse_command(&input).is_some_and(|command| matches!(command, AppCommand::Unknown(_)))
    {
        if let Some(index) = slash_selected {
            if let Some(command) = slash_suggestions(&input).get(index) {
                input = command.command.to_string();
            }
        }
    }

    if !input.is_empty()
        && app.composer.history.last().map(|entry| entry.as_str()) != Some(input.as_str())
    {
        app.composer.history.push(input.clone());
    }
    app.composer.history_index = None;
    app.overlay = Overlay::None;

    if !has_images && is_command_input {
        match parse_command(&input) {
            Some(AppCommand::Quit) => UiAction::Quit,
            Some(AppCommand::Model) => {
                app.composer.clear();
                open_model_picker_overlay(app, runtime);
                UiAction::None
            }
            Some(AppCommand::Settings) => {
                app.composer.clear();
                app.overlay = Overlay::SettingsMenu(SettingsMenuState { selected: 0 });
                UiAction::None
            }
            Some(AppCommand::SettingsAuth) => {
                app.composer.clear();
                open_auth_settings_overlay(app, runtime);
                UiAction::None
            }
            Some(AppCommand::SettingsModel) => {
                app.composer.clear();
                open_model_settings_overlay(app, runtime);
                UiAction::None
            }
            Some(AppCommand::SettingsTheme) => {
                app.composer.clear();
                open_theme_picker_overlay(app, runtime);
                UiAction::None
            }
            Some(AppCommand::SettingsLanguage) => {
                app.composer.clear();
                open_language_picker_overlay(app, runtime);
                UiAction::None
            }
            Some(AppCommand::SettingsTui) => {
                app.composer.clear();
                open_tui_settings_overlay(app, runtime);
                UiAction::None
            }
            Some(AppCommand::Session) => {
                app.composer.clear();
                open_session_picker_overlay(app, runtime);
                UiAction::None
            }
            Some(AppCommand::Compact) => {
                app.composer.clear();
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
                app.composer.clear();
                match runtime.set_prompt_display_enabled(true) {
                    Ok(()) => app.push_note(tr(app.locale, MessageId::DebugPromptEnabled)),
                    Err(error) => app.push_error(error),
                }
                app.refresh_status(runtime);
                UiAction::None
            }
            Some(AppCommand::DebugDisable) => {
                app.composer.clear();
                match runtime.set_prompt_display_enabled(false) {
                    Ok(()) => app.push_note(tr(app.locale, MessageId::DebugPromptDisabled)),
                    Err(error) => app.push_error(error),
                }
                app.refresh_status(runtime);
                UiAction::None
            }
            Some(AppCommand::DebugShow) => {
                app.composer.clear();
                app.transcript
                    .push(TranscriptItem::Note(runtime.system_prompt()));
                UiAction::None
            }
            Some(AppCommand::Auth) => {
                app.composer.clear();
                app.overlay = Overlay::AuthMethod(AuthMethodState { selected: 0 });
                UiAction::None
            }
            Some(AppCommand::Debug) => {
                app.composer.clear();
                app.overlay = Overlay::DebugMenu(DebugMenuState { selected: 0 });
                UiAction::None
            }
            Some(AppCommand::Unknown(command)) => {
                app.composer.clear();
                app.push_error(format!(
                    "{}: {command}",
                    tr(app.locale, MessageId::UnknownCommand)
                ));
                UiAction::None
            }
            None => {
                let images = app.composer.take_images_and_clear();
                UiAction::RunPrompt {
                    prompt: input,
                    images,
                }
            }
        }
    } else {
        let images = app.composer.take_images_and_clear();
        UiAction::RunPrompt {
            prompt: input,
            images,
        }
    }
}

pub(super) fn handle_paste(app: &mut TuiApp, value: &str) {
    if app.is_running {
        if matches!(app.overlay, Overlay::None | Overlay::SlashMenu { .. }) {
            app.insert_paste_text(value);
        }
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
            app.insert_paste_text(value);
        }
        _ => {}
    }
}
