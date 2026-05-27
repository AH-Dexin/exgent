use crossterm::event::{KeyCode, KeyEvent};

use super::state::{Overlay, TuiApp};
use super::suggestions::slash_suggestions;

pub(super) fn handle_running_prompt_key(app: &mut TuiApp, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Enter => {}
        KeyCode::PageUp => app.scroll_transcript_up(10),
        KeyCode::PageDown => app.scroll_transcript_down(10),
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
            move_running_slash_selection(app, -1);
        }
        KeyCode::Down
            if app.composer.images.is_empty()
                && !app.composer.is_multiline()
                && app.composer.input.starts_with('/') =>
        {
            move_running_slash_selection(app, 1);
        }
        KeyCode::Up if app.composer.is_multiline() => app.composer.move_cursor_up(),
        KeyCode::Down if app.composer.is_multiline() => app.composer.move_cursor_down(),
        KeyCode::Up => recall_running_history(app, -1),
        KeyCode::Down => recall_running_history(app, 1),
        KeyCode::Left => app.composer.move_cursor_left(),
        KeyCode::Right => app.composer.move_cursor_right(),
        KeyCode::Home => app.composer.move_cursor_to_start(),
        KeyCode::End => app.composer.move_cursor_to_end(),
        KeyCode::Char(value) => {
            app.composer.insert_char(value);
            app.composer.history_index = None;
            app.sync_slash_menu();
        }
        _ => {}
    }
}

fn move_running_slash_selection(app: &mut TuiApp, delta: isize) {
    let suggestions = slash_suggestions(&app.composer.input);
    if suggestions.is_empty() {
        return;
    }
    let selected = match app.overlay {
        Overlay::SlashMenu { selected } => selected,
        _ => 0,
    };
    let next = if delta < 0 {
        selected.checked_sub(1).unwrap_or(suggestions.len() - 1)
    } else {
        (selected + 1) % suggestions.len()
    };
    app.overlay = Overlay::SlashMenu { selected: next };
}

fn recall_running_history(app: &mut TuiApp, delta: isize) {
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
