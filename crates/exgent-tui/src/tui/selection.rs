use std::io::{self, Write};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Widget},
    Frame,
};

use super::{
    accent, clear_rendered_block, dim, exit_process, finish_inline_terminal, inline_height,
    inline_terminal, print_fitted_terminal_lines, render_active_footer, reset_terminal_viewport,
    strip_ansi, RawModeGuard,
};

pub(super) fn select_with_keys(title: &str, labels: &[&str]) -> io::Result<Option<usize>> {
    if labels.is_empty() {
        return Ok(None);
    }

    let _raw_mode = RawModeGuard::enable()?;
    let mut selected_index = 0usize;
    let mut terminal = match inline_terminal(inline_height(
        u16::try_from(labels.len().saturating_add(4)).unwrap_or(u16::MAX),
    )) {
        Ok(terminal) => terminal,
        Err(_) => {
            reset_terminal_viewport()?;
            return select_with_keys_manual_loop(title, labels);
        }
    };

    loop {
        terminal.draw(|frame| render_key_selector_view(frame, title, labels, selected_index))?;

        let key = match event::read()? {
            Event::Key(key) => key,
            Event::Resize(_, _) => continue,
            _ => continue,
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }

        match key.code {
            KeyCode::Up => {
                selected_index = if selected_index == 0 {
                    labels.len() - 1
                } else {
                    selected_index - 1
                };
            }
            KeyCode::Down => {
                selected_index = (selected_index + 1) % labels.len();
            }
            KeyCode::Enter => {
                finish_inline_terminal(terminal)?;
                return Ok(Some(selected_index));
            }
            KeyCode::Char(' ') => {}
            KeyCode::Esc => {
                finish_inline_terminal(terminal)?;
                return Ok(None);
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let _ = finish_inline_terminal(terminal);
                exit_process();
            }
            _ => {}
        }
    }
}

fn select_with_keys_manual_loop(title: &str, labels: &[&str]) -> io::Result<Option<usize>> {
    let mut selected_index = 0usize;
    let mut rendered_lines = 0usize;
    let mut needs_render = true;

    loop {
        if needs_render {
            rendered_lines =
                render_key_selector_manual(title, labels, selected_index, rendered_lines)?;
            needs_render = false;
        }

        let key = match event::read()? {
            Event::Key(key) => key,
            Event::Resize(_, _) => {
                reset_terminal_viewport()?;
                continue;
            }
            _ => continue,
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }

        match key.code {
            KeyCode::Up => {
                selected_index = if selected_index == 0 {
                    labels.len() - 1
                } else {
                    selected_index - 1
                };
                needs_render = true;
            }
            KeyCode::Down => {
                selected_index = (selected_index + 1) % labels.len();
                needs_render = true;
            }
            KeyCode::Enter => {
                clear_rendered_block(rendered_lines)?;
                return Ok(Some(selected_index));
            }
            KeyCode::Char(' ') => {}
            KeyCode::Esc => {
                clear_rendered_block(rendered_lines)?;
                return Ok(None);
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                clear_rendered_block(rendered_lines)?;
                exit_process();
            }
            _ => {}
        }
    }
}

fn render_key_selector_manual(
    title: &str,
    labels: &[&str],
    selected_index: usize,
    previous_lines: usize,
) -> io::Result<usize> {
    if previous_lines > 0 {
        clear_rendered_block(previous_lines)?;
    }

    let mut lines = Vec::new();
    lines.push(accent(&strip_ansi(title)));
    lines.push(String::new());
    for (index, label) in labels.iter().enumerate() {
        let label = strip_ansi(label);
        let line = if index == selected_index {
            accent(&format!("> {label}"))
        } else {
            format!("  {label}")
        };
        lines.push(line);
    }
    lines.push(String::new());
    lines.push(dim(
        "up/down navigate   space select   enter confirm   escape cancel   ctrl+c exit",
    ));

    let rendered_lines = print_fitted_terminal_lines(&lines)?;
    render_active_footer()?;
    io::stdout().flush()?;
    Ok(rendered_lines)
}

fn render_key_selector_view(
    frame: &mut Frame<'_>,
    title: &str,
    labels: &[&str],
    selected_index: usize,
) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    let hint_height = if area.height >= 3 { 1 } else { 0 };
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(hint_height),
        ])
        .split(area);

    Paragraph::new(Line::from(Span::styled(
        strip_ansi(title),
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )))
    .render(areas[0], frame.buffer_mut());

    let body_height = usize::from(areas[1].height);
    let selected_index = selected_index.min(labels.len().saturating_sub(1));
    let start = selected_index.saturating_sub(body_height.saturating_sub(1));
    let lines = labels
        .iter()
        .enumerate()
        .skip(start)
        .take(body_height)
        .map(|(index, label)| {
            let selected = index == selected_index;
            let marker = if selected { "> " } else { "  " };
            let style = if selected {
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::from(vec![
                Span::styled(marker, style),
                Span::styled(strip_ansi(label), style),
            ])
        })
        .collect::<Vec<_>>();
    Paragraph::new(Text::from(lines)).render(areas[1], frame.buffer_mut());

    if hint_height > 0 {
        Paragraph::new(Line::from(Span::styled(
            "up/down navigate   enter confirm   escape cancel   ctrl+c exit",
            Style::default().fg(Color::DarkGray),
        )))
        .render(areas[2], frame.buffer_mut());
    }
}
