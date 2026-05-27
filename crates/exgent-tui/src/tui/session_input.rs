use crossterm::event::{KeyCode, KeyEvent};
use exgent_core::{tr, AppRuntimeHost, MessageId};

use super::actions::load_recent_messages;
use super::state::*;

type TuiRuntime = AppRuntimeHost;

pub(super) fn handle_session_picker_key(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut SessionPickerState,
) -> UiAction {
    let item_count = state.sessions.len() + 1;
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(item_count - 1);
            app.overlay = Overlay::SessionPicker(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % item_count;
            app.overlay = Overlay::SessionPicker(state.clone());
        }
        KeyCode::Enter => {
            if state.selected == 0 {
                match runtime.start_new_session() {
                    Ok(()) => {
                        app.clear_transcript();
                        app.scroll_transcript_to_bottom();
                        app.refresh_status(runtime);
                        app.push_note(
                            tr(app.locale, MessageId::NewSessionCreated)
                                .replace("{id}", runtime.session_id()),
                        );
                    }
                    Err(error) => app.push_error(error),
                }
            } else if let Some(session) = state.sessions.get(state.selected - 1) {
                match runtime.open_session(&session.path) {
                    Ok(()) => {
                        app.clear_transcript();
                        app.scroll_transcript_to_bottom();
                        app.refresh_status(runtime);
                        app.push_note(
                            tr(app.locale, MessageId::SessionLoaded)
                                .replace("{id}", runtime.session_id()),
                        );
                        load_recent_messages(app, runtime);
                    }
                    Err(error) => app.push_error(error),
                }
            }
            app.overlay = Overlay::None;
        }
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_debug_menu_key(
    app: &mut TuiApp,
    key: KeyEvent,
    state: &mut DebugMenuState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up | KeyCode::Down => {
            state.selected = 0;
            app.overlay = Overlay::DebugMenu(state.clone());
        }
        KeyCode::Enter => {
            app.overlay = Overlay::DebugPrompt(DebugPromptState { selected: 0 });
        }
        _ => {}
    }
    UiAction::None
}

pub(super) fn handle_debug_prompt_key(
    app: &mut TuiApp,
    runtime: &mut TuiRuntime,
    key: KeyEvent,
    state: &mut DebugPromptState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::DebugMenu(DebugMenuState { selected: 0 }),
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(1);
            app.overlay = Overlay::DebugPrompt(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % 2;
            app.overlay = Overlay::DebugPrompt(state.clone());
        }
        KeyCode::Enter => {
            let enabled = state.selected == 0;
            match runtime.set_prompt_display_enabled(enabled) {
                Ok(()) => app.push_note(if enabled {
                    tr(app.locale, MessageId::DebugPromptEnabled)
                } else {
                    tr(app.locale, MessageId::DebugPromptDisabled)
                }),
                Err(error) => app.push_error(error),
            }
            app.refresh_status(runtime);
            app.overlay = Overlay::None;
        }
        _ => {}
    }
    UiAction::None
}
