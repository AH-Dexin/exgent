use exgent_core::{tr, Locale, MessageId, UsageTotals};
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(super) fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let x = area.x.saturating_add(area.width.saturating_sub(width) / 2);
    let y = area
        .y
        .saturating_add(area.height.saturating_sub(height) / 2);
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

pub(super) fn styled_wrapped_lines(
    prefix: &str,
    text: &str,
    width: usize,
    style: Style,
) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for (paragraph_index, paragraph) in text.split('\n').enumerate() {
        let mut wrapped = wrap_plain(paragraph, width.saturating_sub(prefix.width()).max(1));
        if wrapped.is_empty() {
            wrapped.push(String::new());
        }
        for (index, content) in wrapped.into_iter().enumerate() {
            let marker = if paragraph_index == 0 && index == 0 {
                prefix.to_string()
            } else {
                " ".repeat(prefix.width())
            };
            lines.push(Line::from(vec![
                Span::styled(marker, style),
                Span::styled(content, style),
            ]));
        }
    }
    lines
}

fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0usize;

    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if line_width > 0 && line_width + ch_width > width {
            lines.push(line);
            line = String::new();
            line_width = 0;
        }
        line.push(ch);
        line_width += ch_width;
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

pub(super) fn composer_height(width: u16) -> u16 {
    if width < 50 {
        4
    } else {
        3
    }
}

pub(super) fn input_view(input: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(input) <= max_width {
        return input.to_string();
    }
    if max_width <= 1 {
        return ".".to_string();
    }
    format!("…{}", suffix_to_width(input, max_width - 1))
}

pub(super) fn suffix_to_width(text: &str, max_width: usize) -> String {
    let mut chars = Vec::new();
    let mut width = 0usize;
    for ch in text.chars().rev() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        chars.push(ch);
        width += ch_width;
    }
    chars.into_iter().rev().collect()
}

pub(super) fn truncate_plain(text: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }
    let mut result = String::new();
    let mut width = 0usize;
    let target = max_width - 3;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > target {
            break;
        }
        result.push(ch);
        width += ch_width;
    }
    result.push_str("...");
    result
}

pub(super) fn auth_status_text(locale: Locale, is_configured: bool) -> &'static str {
    if is_configured {
        tr(locale, MessageId::AuthConfigured)
    } else {
        tr(locale, MessageId::AuthMissing)
    }
}

pub(super) fn mask_secret(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    "*".repeat(value.chars().count().clamp(1, 40))
}

pub(super) fn format_context_usage(totals: &UsageTotals, context_window: Option<u64>) -> String {
    let Some(context_window) = context_window else {
        return "?/? (auto)".to_string();
    };
    if context_window == 0 {
        return "?/? (auto)".to_string();
    }
    let percent = totals.context_tokens() as f64 * 100.0 / context_window as f64;
    format!("{:.1}%/{} (auto)", percent, format_tokens(context_window))
}

pub(super) fn format_tokens(count: u64) -> String {
    if count < 1_000 {
        return count.to_string();
    }
    if count < 10_000 {
        return format!("{:.1}k", count as f64 / 1_000.0);
    }
    if count < 1_000_000 {
        return format!("{}k", count / 1_000);
    }
    if count < 10_000_000 {
        return format!("{:.1}M", count as f64 / 1_000_000.0);
    }
    format!("{}M", count / 1_000_000)
}
