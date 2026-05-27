use exgent_core::{tr, MessageId};
use ratatui::{
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Block, Borders},
};

use super::formatting::styled_wrapped_lines;
use super::state::{ModelSettingsState, RuntimeActivity, TranscriptItem, TuiApp};

pub(super) fn theme_color(app: &TuiApp) -> Color {
    let theme = app.theme_preview.as_ref().unwrap_or(&app.theme);
    Color::Rgb(theme.rgb.r, theme.rgb.g, theme.rgb.b)
}

pub(super) fn dialog_block<'a>(title: &'a str, accent: Color) -> Block<'a> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED)
        .border_style(Style::default().fg(accent))
}

pub(super) fn selected_style(accent: Color) -> Style {
    Style::default().fg(accent).add_modifier(Modifier::BOLD)
}

pub(super) fn model_settings_rows(
    state: &ModelSettingsState,
    accent: Color,
) -> (Vec<Line<'static>>, usize) {
    let mut rows = Vec::new();
    let mut selected_row = 0usize;
    let mut last_provider = "";
    let selected = state.selected.min(state.models.len().saturating_sub(1));

    for (index, model) in state.models.iter().enumerate() {
        if model.provider != last_provider {
            rows.push(Line::from(Span::styled(
                model.provider.clone(),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )));
            last_provider = &model.provider;
        }

        let is_selected = index == selected;
        if is_selected {
            selected_row = rows.len();
        }
        let checked = state.checked.get(index).copied().unwrap_or(false);
        let style = if is_selected {
            selected_style(accent)
        } else if model.is_enabled {
            Style::default()
        } else {
            Style::default().fg(Color::DarkGray)
        };
        rows.push(Line::from(Span::styled(
            format!(
                "{} {} {}",
                if is_selected { ">" } else { " " },
                if checked { "[*]" } else { "[ ]" },
                model.id
            ),
            style,
        )));
    }

    (rows, selected_row)
}

pub(super) fn item_lines(item: &TranscriptItem, width: usize) -> Vec<Line<'static>> {
    match item {
        TranscriptItem::Welcome(text) => {
            styled_wrapped_lines("", text, width, Style::default().fg(Color::Gray))
        }
        TranscriptItem::User(text) => styled_wrapped_lines(
            "> ",
            text,
            width,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        TranscriptItem::Assistant(text) => {
            if text.is_empty() {
                Vec::new()
            } else {
                styled_wrapped_lines("", text, width, Style::default().fg(Color::White))
            }
        }
        TranscriptItem::Reasoning {
            content,
            duration,
            streaming,
            ..
        } => reasoning_lines(content, *duration, *streaming, width),
        TranscriptItem::Tool(text) => {
            styled_wrapped_lines("tool ", text, width, Style::default().fg(Color::Blue))
        }
        TranscriptItem::Note(text) => {
            styled_wrapped_lines("", text, width, Style::default().fg(Color::Gray))
        }
        TranscriptItem::Error(text) => {
            styled_wrapped_lines("error ", text, width, Style::default().fg(Color::Red))
        }
    }
}

fn reasoning_lines(
    content: &str,
    duration: Option<std::time::Duration>,
    streaming: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled(
            "... thinking",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " {}{}",
                if streaming { "running" } else { "done" },
                duration
                    .map(|duration| format!(" · {:.1}s", duration.as_secs_f64()))
                    .unwrap_or_default()
            ),
            Style::default().fg(Color::DarkGray),
        ),
    ])];

    lines.extend(styled_wrapped_lines(
        "| ",
        content,
        width,
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::ITALIC),
    ));
    lines
}

pub(super) fn runtime_activity_text(app: &TuiApp) -> Option<String> {
    if !app.is_running {
        return None;
    }
    match app.runtime_activity.as_ref()? {
        RuntimeActivity::Thinking => Some(tr(app.locale, MessageId::StatusThinking).to_string()),
        RuntimeActivity::Tool(name) => {
            Some(tr(app.locale, MessageId::RuntimeRunningTool).replace("{tool}", name))
        }
    }
}
