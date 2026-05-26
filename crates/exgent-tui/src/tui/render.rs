use std::cmp;

use exgent_core::{tr, CompatibleModelKind, Locale, MessageId, LANGUAGE_OPTIONS, THEME_PRESETS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::formatting::{
    auth_status_text, centered_rect, format_context_usage, format_tokens, mask_secret,
    styled_wrapped_lines, truncate_plain,
};
use super::render_helpers::{
    dialog_block, item_lines, model_settings_rows, runtime_activity_text, selected_style,
    theme_color,
};
use super::state::*;
use super::suggestions::slash_suggestions;

const SIDEBAR_MIN_WIDTH: u16 = 100;
const SIDEBAR_WIDTH: u16 = 36;
const COMPOSER_MAX_HEIGHT: u16 = 10;
const COMPOSER_PROMPT: &str = "> ";

pub(super) fn render(frame: &mut Frame<'_>, app: &mut TuiApp) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(composer_height(area, &app.composer)),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, vertical[0], app);
    let show_sidebar = area.width >= SIDEBAR_MIN_WIDTH;
    let body_chunks = if show_sidebar {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(SIDEBAR_WIDTH)])
            .split(vertical[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(vertical[1])
    };

    render_transcript(frame, body_chunks[0], app);
    if show_sidebar {
        render_sidebar(frame, body_chunks[1], app);
    }
    render_composer(frame, vertical[2], app);
    render_footer(frame, vertical[3], app);
    render_overlay(frame, area, app);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let line = Line::from(vec![
        Span::styled(
            "Exgent",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("agent", Style::default().fg(Color::Magenta)),
        Span::raw("  "),
        Span::styled(app.model_label.clone(), Style::default().fg(Color::Gray)),
    ]);
    Paragraph::new(line).render(area, frame.buffer_mut());
}

fn render_transcript(frame: &mut Frame<'_>, area: Rect, app: &mut TuiApp) {
    let width = usize::from(area.width).max(1);
    let lines = transcript_lines(app, width);
    let height = usize::from(area.height);
    let (start, scroll) = transcript_visible_start(lines.len(), height, app.transcript_scroll);
    app.clamp_transcript_scroll(scroll);

    let visible = lines
        .into_iter()
        .skip(start)
        .take(height)
        .collect::<Vec<_>>();
    Paragraph::new(Text::from(visible)).render(area, frame.buffer_mut());
}

fn transcript_lines(app: &TuiApp, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for item in &app.transcript {
        let mut item_lines = item_lines(item, width);
        if !item_lines.is_empty() {
            lines.append(&mut item_lines);
            lines.push(Line::raw(""));
        }
    }
    if let Some(activity) = runtime_activity_text(app) {
        lines.extend(styled_wrapped_lines(
            "",
            &activity,
            width,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
        ));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            tr(app.locale, MessageId::EmptyTranscriptHint),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn transcript_visible_start(
    line_count: usize,
    viewport_height: usize,
    scroll: usize,
) -> (usize, usize) {
    let bottom_start = line_count.saturating_sub(viewport_height);
    let scroll = scroll.min(bottom_start);
    (bottom_start.saturating_sub(scroll), scroll)
}

fn render_sidebar(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let inner_width = usize::from(area.width.saturating_sub(2)).max(1);
    let lines = vec![
        Line::from(Span::styled(
            tr(app.locale, MessageId::SidebarStatus),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::raw(format!(
            "{}: {}",
            tr(app.locale, MessageId::SidebarSession),
            truncate_plain(&app.session_id, inner_width)
        )),
        Line::raw(format!(
            "{}: {}",
            tr(app.locale, MessageId::SidebarModel),
            truncate_plain(&app.model_label, inner_width)
        )),
        Line::raw(format!(
            "{}: {}",
            tr(app.locale, MessageId::SidebarReasoning),
            if app.model_reasoning {
                tr(app.locale, MessageId::ReasoningHigh)
            } else {
                tr(app.locale, MessageId::ReasoningOff)
            }
        )),
        Line::raw(""),
        Line::from(Span::styled(
            tr(app.locale, MessageId::SidebarUsage),
            Style::default().fg(Color::Cyan),
        )),
        Line::raw(format!(
            "{}: {}",
            tr(app.locale, MessageId::SidebarInput),
            format_tokens(app.usage.input)
        )),
        Line::raw(format!(
            "{}: {}",
            tr(app.locale, MessageId::SidebarOutput),
            format_tokens(app.usage.output)
        )),
        Line::raw(format!(
            "{}: {}",
            tr(app.locale, MessageId::SidebarCacheRead),
            format_tokens(app.usage.cache_read)
        )),
        Line::raw(format!(
            "{}: {}",
            tr(app.locale, MessageId::SidebarContext),
            format_context_usage(&app.usage, app.model_context_window)
        )),
        Line::raw(""),
        Line::from(Span::styled(
            tr(app.locale, MessageId::SidebarCommands),
            Style::default().fg(Color::Cyan),
        )),
        Line::raw("/model"),
        Line::raw("/auth"),
        Line::raw("/settings"),
        Line::raw("/compact"),
        Line::raw("/quit"),
    ];
    Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::LEFT)
                .border_set(symbols::border::ROUNDED)
                .border_style(Style::default().fg(theme_color(app))),
        )
        .wrap(Wrap { trim: false })
        .render(area, frame.buffer_mut());
}

fn render_composer(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let title = if app.composer.images.is_empty() {
        tr(app.locale, MessageId::ComposerTitle).to_string()
    } else if app.composer.images.len() == 1 {
        format!("{} (1 image)", tr(app.locale, MessageId::ComposerTitle))
    } else {
        format!(
            "{} ({} images)",
            tr(app.locale, MessageId::ComposerTitle),
            app.composer.images.len()
        )
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED)
        .border_style(Style::default().fg(theme_color(app)));
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    let prompt = COMPOSER_PROMPT;
    let prompt_width = UnicodeWidthStr::width(prompt);
    let input_width = usize::from(inner.width).saturating_sub(prompt_width);
    let prompt_style = Style::default().fg(theme_color(app));
    let input_view = composer_input_view(
        &app.composer,
        input_width,
        usize::from(inner.height).max(1),
        prompt_style.add_modifier(Modifier::BOLD),
    );
    let lines = input_view
        .lines
        .into_iter()
        .enumerate()
        .map(|(index, mut spans)| {
            let prefix = if index == 0 {
                prompt.to_string()
            } else {
                " ".repeat(prompt_width)
            };
            let mut line_spans = vec![Span::styled(prefix, prompt_style)];
            line_spans.append(&mut spans);
            Line::from(line_spans)
        })
        .collect::<Vec<_>>();
    Paragraph::new(Text::from(lines)).render(inner, frame.buffer_mut());

    let cursor_x = prompt_width
        .saturating_add(input_view.cursor_x)
        .min(usize::from(inner.width.saturating_sub(1))) as u16;
    let cursor_y = input_view
        .cursor_y
        .min(usize::from(inner.height.saturating_sub(1))) as u16;
    frame.set_cursor_position(Position::new(
        inner.x.saturating_add(cursor_x),
        inner.y.saturating_add(cursor_y),
    ));
}

fn composer_height(area: Rect, composer: &ComposerState) -> u16 {
    let min_height = if area.width < 50 { 4 } else { 3 };
    let max_height = area
        .height
        .saturating_sub(2)
        .max(1)
        .min(COMPOSER_MAX_HEIGHT);
    let inner_width = area.width.saturating_sub(2);
    let input_width =
        usize::from(inner_width).saturating_sub(UnicodeWidthStr::width(COMPOSER_PROMPT));
    let desired = (composer_visual_line_count(composer, input_width) as u16)
        .saturating_add(2)
        .max(min_height);
    desired.min(max_height).max(1)
}

struct ComposerInputView {
    lines: Vec<Vec<Span<'static>>>,
    cursor_x: usize,
    cursor_y: usize,
}

#[derive(Clone)]
struct ComposerDisplayCell {
    text: String,
    width: usize,
    style: Style,
}

enum ComposerDisplayItem {
    Cell(ComposerDisplayCell),
    Newline,
}

#[derive(Default)]
struct ComposerVisualLine {
    spans: Vec<Span<'static>>,
    width: usize,
}

fn composer_input_view(
    composer: &ComposerState,
    max_width: usize,
    max_lines: usize,
    image_style: Style,
) -> ComposerInputView {
    let (lines, cursor_y, cursor_x) = composer_visual_lines(composer, max_width, image_style);
    let total = lines.len().max(1);
    let max_lines = max_lines.max(1);
    let cursor_y = cursor_y.min(total - 1);
    let start = cursor_y
        .saturating_add(1)
        .saturating_sub(max_lines)
        .min(total.saturating_sub(max_lines));
    let visible = lines
        .into_iter()
        .skip(start)
        .take(max_lines)
        .map(|line| line.spans)
        .collect::<Vec<_>>();

    ComposerInputView {
        lines: visible,
        cursor_x,
        cursor_y: cursor_y.saturating_sub(start),
    }
}

fn composer_visual_line_count(composer: &ComposerState, max_width: usize) -> usize {
    composer_visual_lines(composer, max_width, Style::default())
        .0
        .len()
}

fn composer_visual_lines(
    composer: &ComposerState,
    max_width: usize,
    image_style: Style,
) -> (Vec<ComposerVisualLine>, usize, usize) {
    let max_width = max_width.max(1);
    let items = composer_display_items(composer, image_style);
    let cursor = composer.cursor().min(items.len());
    let mut lines = Vec::new();
    let mut line = ComposerVisualLine::default();
    let mut cursor_position = None;

    for (index, item) in items.into_iter().enumerate() {
        match item {
            ComposerDisplayItem::Newline => {
                if cursor == index {
                    cursor_position = Some((lines.len(), line.width));
                }
                lines.push(line);
                line = ComposerVisualLine::default();
            }
            ComposerDisplayItem::Cell(cell) => {
                if line.width > 0 && line.width + cell.width > max_width {
                    lines.push(line);
                    line = ComposerVisualLine::default();
                }
                if cursor == index {
                    cursor_position = Some((lines.len(), line.width));
                }
                line.width = line.width.saturating_add(cell.width);
                line.spans.push(Span::styled(cell.text, cell.style));
            }
        }
    }

    if cursor == composer.cursor().min(composer.items().len()) {
        cursor_position.get_or_insert((lines.len(), line.width));
    }
    lines.push(line);
    let (cursor_y, cursor_x) = cursor_position.unwrap_or((0, 0));
    (lines, cursor_y, cursor_x)
}

fn composer_display_items(
    composer: &ComposerState,
    image_style: Style,
) -> Vec<ComposerDisplayItem> {
    let mut image_index = 0usize;
    composer
        .items()
        .iter()
        .map(|item| match item {
            ComposerItem::Text('\n') => ComposerDisplayItem::Newline,
            ComposerItem::Text(ch) => ComposerDisplayItem::Cell(ComposerDisplayCell {
                text: ch.to_string(),
                width: UnicodeWidthChar::width(*ch).unwrap_or(0),
                style: Style::default(),
            }),
            ComposerItem::Image(_) => {
                image_index += 1;
                let text = format!("[Image #{image_index}]");
                let width = UnicodeWidthStr::width(text.as_str());
                ComposerDisplayItem::Cell(ComposerDisplayCell {
                    text,
                    width,
                    style: image_style,
                })
            }
        })
        .collect()
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let width = usize::from(area.width).max(1);
    let left = truncate_plain(&app.cwd, width / 2);
    let right = format_context_usage(&app.usage, app.model_context_window);
    let spacer = " ".repeat(width.saturating_sub(
        UnicodeWidthStr::width(left.as_str()) + UnicodeWidthStr::width(right.as_str()),
    ));
    Paragraph::new(Line::from(vec![
        Span::styled(left, Style::default().fg(Color::DarkGray)),
        Span::raw(spacer),
        Span::styled(right, Style::default().fg(Color::DarkGray)),
    ]))
    .render(area, frame.buffer_mut());
}

fn render_overlay(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let accent = theme_color(app);
    match &app.overlay {
        Overlay::None => {}
        Overlay::SlashMenu { selected } => render_slash_menu(frame, area, app, *selected),
        Overlay::ModelPicker(picker) => {
            render_model_picker(frame, area, picker, app.locale, accent)
        }
        Overlay::SettingsMenu(state) => {
            render_settings_menu(frame, area, state, app.locale, accent)
        }
        Overlay::AuthSettings(state) => {
            render_auth_settings(frame, area, state, app.locale, accent)
        }
        Overlay::AuthAction(state) => render_action_menu(
            frame,
            area,
            tr(app.locale, MessageId::DialogProviderAction),
            &[
                tr(app.locale, MessageId::ActionEnable),
                tr(app.locale, MessageId::ActionDisable),
                tr(app.locale, MessageId::ActionRemove),
            ],
            state.selected,
            accent,
        ),
        Overlay::ModelSettings(state) => {
            render_model_settings(frame, area, state, app.locale, accent)
        }
        Overlay::ModelAction(state) => render_action_menu(
            frame,
            area,
            tr(app.locale, MessageId::DialogModelAction),
            &[
                tr(app.locale, MessageId::ActionEnable),
                tr(app.locale, MessageId::ActionDisable),
            ],
            state.selected,
            accent,
        ),
        Overlay::ThemePicker(state) => render_theme_picker(frame, area, app, state),
        Overlay::CustomTheme(state) => render_custom_theme_form(frame, area, app, state),
        Overlay::LanguagePicker(state) => render_language_picker(frame, area, app, state),
        Overlay::SessionPicker(state) => {
            render_session_picker(frame, area, state, app.locale, accent)
        }
        Overlay::DebugMenu(state) => render_debug_menu(frame, area, state, app.locale, accent),
        Overlay::DebugPrompt(state) => {
            render_debug_prompt_menu(frame, area, state, app.locale, accent)
        }
        Overlay::AuthMethod(state) => {
            render_auth_method_menu(frame, area, state, app.locale, accent)
        }
        Overlay::ApiKeyProvider(state) => {
            render_api_key_provider_picker(frame, area, state, app.locale, accent)
        }
        Overlay::ApiKeyInput(state) => render_api_key_input(frame, area, state, app.locale, accent),
        Overlay::CustomModelKind(state) => {
            render_custom_model_kind_picker(frame, area, state, app.locale, accent)
        }
        Overlay::AddModelForm(state) => {
            render_add_model_form(frame, area, state, app.locale, accent)
        }
        Overlay::SubscriptionProvider(state) => {
            render_subscription_provider_picker(frame, area, state, app.locale, accent)
        }
        Overlay::AuthProgress(state) => render_auth_progress(frame, area, state, accent),
    }
}

fn render_slash_menu(frame: &mut Frame<'_>, area: Rect, app: &TuiApp, selected: usize) {
    let suggestions = slash_suggestions(&app.composer.input);
    if suggestions.is_empty() {
        return;
    }
    let accent = theme_color(app);
    let width = cmp::min(area.width.saturating_sub(4), 72).max(20);
    let height = cmp::min(suggestions.len() as u16 + 2, area.height.saturating_sub(4)).max(3);
    let popup = Rect::new(
        area.x.saturating_add(2),
        area.y
            .saturating_add(area.height.saturating_sub(height).saturating_sub(3)),
        width,
        height,
    );
    Clear.render(popup, frame.buffer_mut());
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = suggestions
        .iter()
        .enumerate()
        .take(usize::from(height.saturating_sub(2)))
        .map(|(index, command)| {
            let is_selected = index == selected.min(suggestions.len().saturating_sub(1));
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let label = format!(
                "{:<18} {}",
                command.command,
                tr(app.locale, command.description_id)
            );
            Line::from(Span::styled(truncate_plain(&label, inner_width), style))
        })
        .collect::<Vec<_>>();
    Paragraph::new(Text::from(lines))
        .block(dialog_block("/", accent))
        .render(popup, frame.buffer_mut());
}

fn render_model_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    picker: &ModelPickerState,
    locale: Locale,
    accent: Color,
) {
    let width = cmp::min(area.width.saturating_sub(4), 88).max(28);
    let height = cmp::min(
        picker.models.len() as u16 + 2,
        area.height.saturating_sub(4),
    )
    .max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let selected = picker.selected.min(picker.models.len().saturating_sub(1));
    let start = selected.saturating_sub(body_height.saturating_sub(1));
    let lines = picker
        .models
        .iter()
        .enumerate()
        .skip(start)
        .take(body_height)
        .map(|(index, model)| {
            let is_selected = index == selected;
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let current = if model.is_current { " *" } else { "" };
            let label = format!("{}/{}  {}{}", model.provider, model.id, model.name, current);
            Line::from(Span::styled(
                truncate_plain(&label, usize::from(width.saturating_sub(2))),
                style,
            ))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(tr(locale, MessageId::DialogModel), accent))
        .render(popup, frame.buffer_mut());
}

fn render_settings_menu(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &SettingsMenuState,
    locale: Locale,
    accent: Color,
) {
    let labels = [
        tr(locale, MessageId::SettingsAuth),
        tr(locale, MessageId::SettingsModel),
        tr(locale, MessageId::SettingsTheme),
        tr(locale, MessageId::SettingsLanguage),
    ];
    render_action_menu(
        frame,
        area,
        tr(locale, MessageId::DialogSettings),
        &labels,
        state.selected,
        accent,
    );
}

fn render_auth_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AuthSettingsState,
    locale: Locale,
    accent: Color,
) {
    let width = cmp::min(area.width.saturating_sub(4), 72).max(30);
    let height = cmp::min(
        state.providers.len() as u16 + 2,
        area.height.saturating_sub(4),
    )
    .max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let selected = state.selected.min(state.providers.len().saturating_sub(1));
    let start = selected.saturating_sub(body_height.saturating_sub(1));
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = state
        .providers
        .iter()
        .enumerate()
        .skip(start)
        .take(body_height)
        .map(|(index, provider)| {
            let is_selected = index == selected;
            let checked = state.checked.get(index).copied().unwrap_or(false);
            let style = if is_selected {
                selected_style(accent)
            } else if provider.is_enabled {
                Style::default()
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let label = format!(
                "{} {} {}",
                if is_selected { ">" } else { " " },
                if checked { "[*]" } else { "[ ]" },
                provider.provider
            );
            Line::from(Span::styled(truncate_plain(&label, inner_width), style))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(
            tr(locale, MessageId::DialogSettingsAuth),
            accent,
        ))
        .render(popup, frame.buffer_mut());
}

fn render_model_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &ModelSettingsState,
    locale: Locale,
    accent: Color,
) {
    let (rows, selected_row) = model_settings_rows(state, accent);
    let width = cmp::min(area.width.saturating_sub(4), 88).max(32);
    let height = cmp::min(rows.len() as u16 + 2, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let start = selected_row.saturating_sub(body_height.saturating_sub(1));
    let visible = rows
        .into_iter()
        .skip(start)
        .take(body_height)
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(visible))
        .block(dialog_block(
            tr(locale, MessageId::DialogSettingsModel),
            accent,
        ))
        .render(popup, frame.buffer_mut());
}

fn render_action_menu(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    labels: &[&str],
    selected: usize,
    accent: Color,
) {
    let longest = labels.iter().map(|label| label.width()).max().unwrap_or(16);
    let width = cmp::min(area.width.saturating_sub(4), (longest + 8) as u16).max(24);
    let height = cmp::min(labels.len() as u16 + 2, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let is_selected = index == selected.min(labels.len().saturating_sub(1));
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let text = format!("{} {label}", if is_selected { ">" } else { " " });
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .collect::<Vec<_>>();
    Paragraph::new(Text::from(lines))
        .block(dialog_block(title, accent))
        .render(popup, frame.buffer_mut());
}

fn render_theme_picker(frame: &mut Frame<'_>, area: Rect, app: &TuiApp, state: &ThemePickerState) {
    let item_count = THEME_PRESETS.len() + 1;
    let accent = theme_color(app);
    let width = cmp::min(area.width.saturating_sub(4), 72).max(36);
    let height = cmp::min(item_count as u16 + 2, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let selected = state.selected.min(item_count.saturating_sub(1));
    let start = selected.saturating_sub(body_height.saturating_sub(1));
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = (0..item_count)
        .skip(start)
        .take(body_height)
        .map(|index| {
            let is_selected = index == selected;
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let label = if let Some(preset) = THEME_PRESETS.get(index) {
                let current = if app.theme.name == preset.name && app.theme.rgb == preset.rgb {
                    " *"
                } else {
                    ""
                };
                format!(
                    "{}  rgb({}, {}, {}){}",
                    preset.name, preset.rgb.r, preset.rgb.g, preset.rgb.b, current
                )
            } else {
                let current = if app.theme.name == "custom" { " *" } else { "" };
                format!("{}{}", tr(app.locale, MessageId::CustomRgb), current)
            };
            let text = format!("{} {label}", if is_selected { ">" } else { " " });
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(tr(app.locale, MessageId::DialogTheme), accent))
        .render(popup, frame.buffer_mut());
}

fn render_custom_theme_form(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &TuiApp,
    state: &CustomThemeState,
) {
    let accent = theme_color(app);
    let width = cmp::min(area.width.saturating_sub(4), 52).max(32);
    let height = cmp::min(6, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let inner_width = usize::from(width.saturating_sub(2));
    let fields = [
        (tr(app.locale, MessageId::FieldRed), state.red.as_str()),
        (tr(app.locale, MessageId::FieldGreen), state.green.as_str()),
        (tr(app.locale, MessageId::FieldBlue), state.blue.as_str()),
    ];
    let lines = fields
        .iter()
        .enumerate()
        .map(|(index, (label, value))| {
            let is_selected = index == state.field.min(2);
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let value = if value.is_empty() {
                tr(app.locale, MessageId::EmptyValue)
            } else {
                value
            };
            let text = format!(
                "{} {:<5} {}",
                if is_selected { ">" } else { " " },
                label,
                value
            );
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .chain([Line::from(Span::styled(
            tr(app.locale, MessageId::CustomRgbHint),
            Style::default().fg(Color::DarkGray),
        ))])
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(tr(app.locale, MessageId::CustomRgb), accent))
        .render(popup, frame.buffer_mut());
}

fn render_language_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &TuiApp,
    state: &LanguagePickerState,
) {
    let accent = theme_color(app);
    let width = cmp::min(area.width.saturating_sub(4), 52).max(28);
    let height = cmp::min(
        LANGUAGE_OPTIONS.len() as u16 + 2,
        area.height.saturating_sub(4),
    )
    .max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let selected = state.selected.min(LANGUAGE_OPTIONS.len().saturating_sub(1));
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = LANGUAGE_OPTIONS
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let is_selected = index == selected;
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let current = if app.locale == option.locale {
                " *"
            } else {
                ""
            };
            let text = format!(
                "{} {}{}",
                if is_selected { ">" } else { " " },
                option.label,
                current
            );
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(
            tr(app.locale, MessageId::DialogLanguage),
            accent,
        ))
        .render(popup, frame.buffer_mut());
}

fn render_session_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &SessionPickerState,
    locale: Locale,
    accent: Color,
) {
    let item_count = state.sessions.len() + 1;
    let width = cmp::min(area.width.saturating_sub(4), 96).max(36);
    let height = cmp::min(item_count as u16 + 2, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let selected = state.selected.min(item_count.saturating_sub(1));
    let start = selected.saturating_sub(body_height.saturating_sub(1));
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = (0..item_count)
        .skip(start)
        .take(body_height)
        .map(|index| {
            let is_selected = index == selected;
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let label = if index == 0 {
                tr(locale, MessageId::NewSession).to_string()
            } else {
                let session = &state.sessions[index - 1];
                let preview = session
                    .preview
                    .as_deref()
                    .map(|preview| format!("  {preview}"))
                    .unwrap_or_default();
                format!(
                    "{}  {}={}  {}{}",
                    session.id,
                    tr(locale, MessageId::LabelMessages),
                    session.message_count,
                    session.cwd,
                    preview
                )
            };
            let text = format!("{} {label}", if is_selected { ">" } else { " " });
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(tr(locale, MessageId::DialogSession), accent))
        .render(popup, frame.buffer_mut());
}

fn render_debug_menu(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &DebugMenuState,
    locale: Locale,
    accent: Color,
) {
    render_action_menu(
        frame,
        area,
        tr(locale, MessageId::DialogDebug),
        &[tr(locale, MessageId::DebugPrompt)],
        state.selected,
        accent,
    );
}

fn render_debug_prompt_menu(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &DebugPromptState,
    locale: Locale,
    accent: Color,
) {
    render_action_menu(
        frame,
        area,
        tr(locale, MessageId::DialogDebugPrompt),
        &[
            tr(locale, MessageId::ActionEnable),
            tr(locale, MessageId::ActionDisable),
        ],
        state.selected,
        accent,
    );
}

fn render_auth_method_menu(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AuthMethodState,
    locale: Locale,
    accent: Color,
) {
    render_action_menu(
        frame,
        area,
        tr(locale, MessageId::DialogAuthentication),
        &[
            tr(locale, MessageId::AuthSubscription),
            tr(locale, MessageId::AuthApiKey),
        ],
        state.selected,
        accent,
    );
}

fn render_api_key_provider_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &ApiKeyProviderState,
    locale: Locale,
    accent: Color,
) {
    let item_count = state.providers.len() + 1;
    let width = cmp::min(area.width.saturating_sub(4), 72).max(36);
    let height = cmp::min(item_count as u16 + 2, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let selected = state.selected.min(item_count.saturating_sub(1));
    let start = selected.saturating_sub(body_height.saturating_sub(1));
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = (0..item_count)
        .skip(start)
        .take(body_height)
        .map(|index| {
            let is_selected = index == selected;
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let label = if index == state.providers.len() {
                tr(locale, MessageId::CustomModel).to_string()
            } else {
                let provider = &state.providers[index];
                format!(
                    "{}  {}",
                    provider.provider,
                    auth_status_text(locale, provider.has_token)
                )
            };
            let text = format!("{} {label}", if is_selected { ">" } else { " " });
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(tr(locale, MessageId::DialogApiKey), accent))
        .render(popup, frame.buffer_mut());
}

fn render_custom_model_kind_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &CustomModelKindState,
    locale: Locale,
    accent: Color,
) {
    render_action_menu(
        frame,
        area,
        tr(locale, MessageId::DialogCustomModel),
        &[
            tr(locale, MessageId::CompatibleOpenAi),
            tr(locale, MessageId::CompatibleAnthropic),
            tr(locale, MessageId::CompatibleGoogle),
        ],
        state.selected,
        accent,
    );
}

fn render_api_key_input(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &ApiKeyInputState,
    locale: Locale,
    accent: Color,
) {
    let width = cmp::min(area.width.saturating_sub(4), 72).max(36);
    let height = cmp::min(7, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let inner_width = usize::from(width.saturating_sub(2));
    let api_key = if state.value.is_empty() {
        tr(locale, MessageId::FieldApiKeyHint).to_string()
    } else {
        mask_secret(&state.value)
    };
    let url = state.base_url.as_deref().unwrap_or("-");
    let title = format!(
        "{}: {}",
        tr(locale, MessageId::FieldProvider),
        state.provider
    );
    let lines = [
        format!("{}: {url}", tr(locale, MessageId::FieldUrl)),
        String::new(),
        format!("API_KEY: {api_key}"),
        String::new(),
        tr(locale, MessageId::EnterSavesEscapeCancels).to_string(),
    ]
    .into_iter()
    .map(|line| Line::raw(truncate_plain(&line, inner_width)))
    .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(&title, accent))
        .render(popup, frame.buffer_mut());
}

fn render_add_model_form(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AddModelFormState,
    locale: Locale,
    accent: Color,
) {
    let width = cmp::min(area.width.saturating_sub(4), 82).max(40);
    let height = cmp::min(8, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let inner_width = usize::from(width.saturating_sub(2));
    let masked_api_key = mask_secret(&state.api_key);
    let fields = [
        (
            tr(locale, MessageId::FieldProvider),
            state.provider.as_str(),
        ),
        (tr(locale, MessageId::FieldModelId), state.model_id.as_str()),
        (tr(locale, MessageId::FieldBaseUrl), state.base_url.as_str()),
        (tr(locale, MessageId::FieldApiKey), masked_api_key.as_str()),
    ];
    let body_height = usize::from(height.saturating_sub(2));
    let start = state.field.saturating_sub(body_height.saturating_sub(1));
    let lines = fields
        .iter()
        .enumerate()
        .skip(start)
        .take(body_height)
        .map(|(index, (label, value))| {
            let is_selected = index == state.field.min(3);
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let value = if value.is_empty() {
                tr(locale, MessageId::EmptyValue)
            } else {
                value
            };
            let text = format!(
                "{} {:<9} {}",
                if is_selected { ">" } else { " " },
                label,
                value
            );
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .collect::<Vec<_>>();

    let title = format!(
        "{} ({})",
        tr(locale, MessageId::DialogCustomModel),
        match state.kind {
            CompatibleModelKind::OpenAi => tr(locale, MessageId::CompatibleOpenAi),
            CompatibleModelKind::Anthropic => tr(locale, MessageId::CompatibleAnthropic),
            CompatibleModelKind::Google => tr(locale, MessageId::CompatibleGoogle),
        }
    );

    Paragraph::new(Text::from(lines))
        .block(dialog_block(&title, accent))
        .render(popup, frame.buffer_mut());
}

fn render_subscription_provider_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &SubscriptionProviderState,
    locale: Locale,
    accent: Color,
) {
    let width = cmp::min(area.width.saturating_sub(4), 72).max(36);
    let height = cmp::min(
        state.providers.len() as u16 + 2,
        area.height.saturating_sub(4),
    )
    .max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let selected = state.selected.min(state.providers.len().saturating_sub(1));
    let start = selected.saturating_sub(body_height.saturating_sub(1));
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = state
        .providers
        .iter()
        .enumerate()
        .skip(start)
        .take(body_height)
        .map(|(index, provider)| {
            let is_selected = index == selected;
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let label = format!(
                "{}  {}",
                provider.name,
                auth_status_text(locale, provider.has_subscription)
            );
            let text = format!("{} {label}", if is_selected { ">" } else { " " });
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(
            tr(locale, MessageId::DialogSubscription),
            accent,
        ))
        .render(popup, frame.buffer_mut());
}

fn render_auth_progress(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AuthProgressState,
    accent: Color,
) {
    let width = cmp::min(area.width.saturating_sub(4), 96).max(40);
    let inner_width = usize::from(width.saturating_sub(2));

    // URL lines wrap across multiple rows; compute true height before sizing the popup
    let content_height: u16 = state
        .lines
        .iter()
        .map(|line| {
            if (line.starts_with("https://") || line.starts_with("http://")) && inner_width > 0 {
                let w = UnicodeWidthStr::width(line.as_str());
                w.div_ceil(inner_width).max(1) as u16
            } else {
                1
            }
        })
        .sum::<u16>()
        .max(1);

    let height = cmp::min(content_height + 2, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let mut lines: Vec<Line<'_>> = Vec::new();
    let mut remaining = body_height;

    for line in &state.lines {
        if remaining == 0 {
            break;
        }
        if (line.starts_with("https://") || line.starts_with("http://")) && inner_width > 0 {
            let w = UnicodeWidthStr::width(line.as_str());
            let url_rows = w.div_ceil(inner_width).max(1);
            lines.push(Line::raw(line.clone()));
            remaining = remaining.saturating_sub(url_rows);
        } else {
            lines.push(Line::raw(truncate_plain(line, inner_width)));
            remaining -= 1;
        }
    }

    Paragraph::new(Text::from(lines))
        .block(dialog_block(&state.title, accent))
        .wrap(Wrap { trim: false })
        .render(popup, frame.buffer_mut());
}

#[cfg(test)]
mod tests {
    use super::*;
    use exgent_core::ImageContent;

    fn span_text(view: &ComposerInputView) -> String {
        view.lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn composer_view_renders_inline_image_blocks() {
        let mut composer = ComposerState::default();
        composer.insert_image(ImageContent::new("one", "image/png"));
        composer.insert_image(ImageContent::new("two", "image/png"));

        let view = composer_input_view(&composer, 80, 4, Style::default());

        assert_eq!(span_text(&view), "[Image #1][Image #2]");
        assert_eq!(view.cursor_x, "[Image #1][Image #2]".width());
        assert_eq!(view.cursor_y, 0);
    }

    #[test]
    fn composer_view_cursor_moves_over_image_blocks() {
        let mut composer = ComposerState::default();
        composer.insert_char('a');
        composer.insert_image(ImageContent::new("one", "image/png"));
        composer.insert_char('b');
        composer.move_cursor_left();
        composer.move_cursor_left();

        let view = composer_input_view(&composer, 80, 4, Style::default());

        assert_eq!(span_text(&view), "a[Image #1]b");
        assert_eq!(view.cursor_x, "a".width());
        assert_eq!(view.cursor_y, 0);
    }

    #[test]
    fn composer_view_renders_multiline_paste() {
        let mut composer = ComposerState::default();
        composer.insert_str("first\nsecond");

        let view = composer_input_view(&composer, 80, 4, Style::default());

        assert_eq!(span_text(&view), "first\nsecond");
        assert_eq!(view.cursor_x, "second".width());
        assert_eq!(view.cursor_y, 1);
    }

    #[test]
    fn composer_view_scrolls_to_cursor_line() {
        let mut composer = ComposerState::default();
        composer.insert_str("one\ntwo\nthree");

        let view = composer_input_view(&composer, 80, 2, Style::default());

        assert_eq!(span_text(&view), "two\nthree");
        assert_eq!(view.cursor_x, "three".width());
        assert_eq!(view.cursor_y, 1);
    }

    #[test]
    fn transcript_visible_start_scrolls_from_bottom() {
        assert_eq!(transcript_visible_start(20, 5, 0), (15, 0));
        assert_eq!(transcript_visible_start(20, 5, 3), (12, 3));
        assert_eq!(transcript_visible_start(20, 5, usize::MAX), (0, 15));
        assert_eq!(transcript_visible_start(3, 5, 10), (0, 0));
    }
}
