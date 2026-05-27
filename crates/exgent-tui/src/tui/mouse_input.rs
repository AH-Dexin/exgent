use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::clipboard_text::write_clipboard_text;
use super::render::{selected_transcript_text, transcript_area, transcript_position_at};
use super::state::{TranscriptSelection, TuiApp};

const MOUSE_WHEEL_LINES: usize = 3;

pub(super) fn handle_mouse(app: &mut TuiApp, terminal_area: Rect, mouse: MouseEvent) -> bool {
    if !app.tui_settings.mouse_selection {
        return false;
    }

    let area = transcript_area(terminal_area, &app.composer);
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) if contains(area, mouse.column, mouse.row) => {
            if let Some(position) = transcript_position_at(app, area, mouse.column, mouse.row) {
                app.transcript_selection = Some(TranscriptSelection::new(position));
                return true;
            }
        }
        MouseEventKind::Drag(MouseButton::Left)
            if app
                .transcript_selection
                .as_ref()
                .is_some_and(|selection| selection.dragging) =>
        {
            if let Some(position) = transcript_position_at(app, area, mouse.column, mouse.row) {
                if let Some(selection) = app.transcript_selection.as_mut() {
                    selection.moved = selection.moved || selection.head != position;
                    selection.head = position;
                }
                return true;
            }
        }
        MouseEventKind::Up(MouseButton::Left)
            if app
                .transcript_selection
                .as_ref()
                .is_some_and(|selection| selection.dragging) =>
        {
            if let Some(position) = transcript_position_at(app, area, mouse.column, mouse.row) {
                if let Some(selection) = app.transcript_selection.as_mut() {
                    selection.moved = selection.moved || selection.head != position;
                    selection.head = position;
                    selection.dragging = false;
                }
            }
            if app
                .transcript_selection
                .as_ref()
                .is_some_and(|selection| !selection.moved)
            {
                app.transcript_selection = None;
                return true;
            }
            let text = selected_transcript_text(app, usize::from(area.width).max(1));
            if !text.is_empty() {
                let _ = write_clipboard_text(&text);
            }
            return true;
        }
        MouseEventKind::ScrollUp if contains(area, mouse.column, mouse.row) => {
            app.scroll_transcript_up(MOUSE_WHEEL_LINES);
            return true;
        }
        MouseEventKind::ScrollDown if contains(area, mouse.column, mouse.row) => {
            app.scroll_transcript_down(MOUSE_WHEEL_LINES);
            return true;
        }
        _ => {}
    }

    false
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}
