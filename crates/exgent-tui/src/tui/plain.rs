use std::{
    io::{self, IsTerminal, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crossterm::{
    cursor,
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{self, ClearType},
};
use exgent_core::{
    tr, AgentEvent, AgentSessionEvent, AppRuntimeHost, Locale, ModelMenuItem, UsageTotals,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Widget},
    Frame, Terminal, TerminalOptions, Viewport,
};
use unicode_width::UnicodeWidthChar;

use crate::commands::{parse_command, AppCommand, COMMAND_HELP};

use super::paste_burst::{FlushResult, PasteBurst, PasteBurstAction};
use super::paste_text::{prepare_paste_text, truncate_large_paste_fallback};
use super::{
    plain_auth::open_auth_menu,
    plain_events::{event_has_visible_output, EventRenderer},
    selection::select_with_keys,
    settings,
};

static ACTIVE_FOOTER: OnceLock<Mutex<Option<FooterLines>>> = OnceLock::new();
static RATATUI_INLINE_DISABLED: AtomicBool = AtomicBool::new(false);
const TERMINAL_VIEWPORT_RESET: &str = "\x1b[r\x1b[?6l";
type InlineTerminal = Terminal<CrosstermBackend<io::Stdout>>;
type TuiRuntime = AppRuntimeHost;

pub(super) fn run(runtime: &mut TuiRuntime) -> io::Result<()> {
    print_startup(runtime)?;

    let mut line = String::new();
    let mut input_history = Vec::new();

    loop {
        line = if io::stdin().is_terminal() && io::stdout().is_terminal() {
            let footer = footer_lines(runtime, terminal_width());
            set_active_footer(Some(footer.clone()));
            match read_prompt_line(&format!("{} ", bold(">")), &input_history, Some(&footer))? {
                Some(line) => line,
                None => continue,
            }
        } else {
            print!("{} ", bold(">"));
            io::stdout().flush()?;
            line.clear();
            let bytes = io::stdin().read_line(&mut line)?;
            if bytes == 0 {
                break;
            }
            line.clone()
        };

        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if input_history.last().map(|entry| entry.as_str()) != Some(input) {
            input_history.push(input.to_string());
        }

        if handle_command(runtime, input)? {
            continue;
        }

        let show_system_prompt = runtime.prompt_display_enabled();
        if show_system_prompt {
            print_prompt_preview(runtime)?;
        }
        let mut renderer = EventRenderer::new(
            true,
            io::stdin().is_terminal() && io::stdout().is_terminal(),
        );
        let spinner_footer = (io::stdin().is_terminal() && io::stdout().is_terminal())
            .then(|| footer_lines(runtime, terminal_width()));
        let mut spinner = ThinkingSpinner::start(spinner_footer);
        let mut streaming_error = false;
        match runtime.run_prompt_events(input, &mut |event| {
            if let AgentSessionEvent::Agent(event) = event {
                if matches!(&event, AgentEvent::Error { .. }) {
                    streaming_error = true;
                }
                if event_has_visible_output(&event, true) {
                    spinner.stop();
                }
                renderer.render(event);
            }
        }) {
            Ok(()) => {}
            Err(error) => {
                spinner.stop();
                if !streaming_error {
                    eprintln!("error: {error}");
                }
            }
        }
        spinner.stop();
    }

    Ok(())
}

fn print_startup(runtime: &TuiRuntime) -> io::Result<()> {
    clear_screen();

    let directory = current_directory_label(runtime);
    let rows = [
        format!(">  Exgent ({})", env!("CARGO_PKG_VERSION")),
        format!("model: {}", runtime.model_label()),
        format!("directory: {directory}"),
        format!("session: {}", runtime.session_id()),
    ];
    print_card(&rows)?;
    println!();
    io::stdout().flush()
}

fn print_card(rows: &[String]) -> io::Result<()> {
    let content_width = card_content_width(rows);
    let top_border = format!("+{}+", "-".repeat(content_width + 2));
    let bottom_border = top_border.clone();

    println!("{}", dim(&top_border));
    for (index, row) in rows.iter().enumerate() {
        let row = truncate_to_width(row, content_width);
        let styled = match index {
            0 => bold(&row),
            2 => accent(&row),
            _ => row.to_string(),
        };
        println!(
            "{} {}{} {}",
            dim("|"),
            styled,
            " ".repeat(content_width.saturating_sub(visible_width(&row))),
            dim("|")
        );
    }
    println!("{}", dim(&bottom_border));
    Ok(())
}

fn card_content_width(rows: &[String]) -> usize {
    let preferred_width = rows
        .iter()
        .map(|row| visible_width(row))
        .max()
        .unwrap_or(0)
        .max(48);
    let terminal_width = terminal::size()
        .map(|(columns, _)| usize::from(columns))
        .unwrap_or(80);
    let max_content_width = terminal_width.saturating_sub(4).max(1);
    preferred_width.min(max_content_width)
}

fn truncate_to_width(text: &str, max_width: usize) -> String {
    if visible_width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }

    let ellipsis = "...";
    let mut truncated = String::new();
    let mut width = 0usize;
    let target_width = max_width - visible_width(ellipsis);
    let mut chars = text.chars().peekable();
    let mut saw_escape = false;
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            saw_escape = true;
            truncated.push(ch);
            if let Some(ch) = chars.next() {
                truncated.push(ch);
            }
            for ch in chars.by_ref() {
                truncated.push(ch);
                if ch.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }

        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > target_width {
            break;
        }
        truncated.push(ch);
        width += ch_width;
    }
    truncated.push_str(ellipsis);
    if saw_escape {
        truncated.push_str("\x1b[0m");
    }
    truncated
}

fn print_prompt_preview(runtime: &TuiRuntime) -> io::Result<()> {
    println!("{}", dim("system prompt:"));
    println!("{}", runtime.system_prompt());
    println!("{}", dim("end system prompt"));
    io::stdout().flush()
}

fn status_line(runtime: &TuiRuntime, width: usize) -> String {
    let totals = runtime.usage_totals();
    let model = runtime.model_status();
    let mut parts = Vec::new();
    if totals.input > 0 {
        parts.push(format!("↑{}", format_tokens(totals.input)));
    }
    if totals.output > 0 {
        parts.push(format!("↓{}", format_tokens(totals.output)));
    }
    if totals.cache_read > 0 {
        parts.push(format!("R{}", format_tokens(totals.cache_read)));
    }
    if totals.cache_write > 0 {
        parts.push(format!("W{}", format_tokens(totals.cache_write)));
    }
    if totals.cost > 0.0 {
        parts.push(format!("${:.3}", totals.cost));
    }

    let context_window = model.as_ref().and_then(|model| model.context_window);
    parts.push(format_context_usage(totals, context_window));
    let left = parts.join(" ");
    let right = model
        .map(|model| {
            let mut label = format!("({}) {}", model.provider, model.id);
            if model.reasoning {
                label.push_str(" • high");
            }
            label
        })
        .unwrap_or_else(|| "no-model".to_string());

    fit_status_parts(&left, &right, width)
}

#[derive(Clone, Debug)]
struct FooterLines {
    path: String,
    status: String,
}

impl FooterLines {
    fn height() -> u16 {
        3
    }
}

fn footer_lines(runtime: &TuiRuntime, width: usize) -> FooterLines {
    FooterLines {
        path: truncate_to_width(&current_directory_label(runtime), width),
        status: status_line(runtime, width),
    }
}

pub(super) fn refresh_active_footer(runtime: &TuiRuntime) {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        set_active_footer(Some(footer_lines(runtime, terminal_width())));
    }
}

fn active_footer_cell() -> &'static Mutex<Option<FooterLines>> {
    ACTIVE_FOOTER.get_or_init(|| Mutex::new(None))
}

fn set_active_footer(footer: Option<FooterLines>) {
    if let Ok(mut active_footer) = active_footer_cell().lock() {
        *active_footer = footer;
    }
}

fn active_footer() -> Option<FooterLines> {
    active_footer_cell()
        .lock()
        .ok()
        .and_then(|footer| footer.clone())
}

pub(super) fn render_active_footer() -> io::Result<()> {
    let footer = active_footer();
    render_footer(footer.as_ref())
}

fn terminal_width() -> usize {
    terminal::size()
        .map(|(columns, _)| usize::from(columns))
        .unwrap_or(80)
}

fn terminal_height() -> u16 {
    terminal::size().map(|(_, rows)| rows).unwrap_or(24)
}

fn print_terminal_line(line: impl AsRef<str>) -> io::Result<()> {
    let mut stdout = io::stdout();
    write!(stdout, "{}\r\n", line.as_ref())
}

pub(super) fn print_fitted_terminal_line(line: impl AsRef<str>) -> io::Result<()> {
    print_terminal_line(truncate_to_width(line.as_ref(), terminal_width()))
}

pub(super) fn print_fitted_terminal_lines(lines: &[String]) -> io::Result<usize> {
    let line_budget = body_line_budget_from_cursor();
    let rendered_lines = lines.len().min(line_budget);
    if rendered_lines == 0 {
        return Ok(0);
    }

    for line in lines.iter().take(rendered_lines.saturating_sub(1)) {
        print_fitted_terminal_line(line)?;
    }

    let last = if lines.len() > rendered_lines {
        dim("...")
    } else {
        lines[rendered_lines - 1].clone()
    };
    print_fitted_terminal_line(last)?;
    Ok(rendered_lines)
}

pub(super) fn print_terminal_newline() -> io::Result<()> {
    let mut stdout = io::stdout();
    write!(stdout, "\r\n")?;
    stdout.flush()
}

pub(super) fn inline_terminal(height: u16) -> io::Result<InlineTerminal> {
    if RATATUI_INLINE_DISABLED.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "ratatui inline viewport disabled for this terminal",
        ));
    }

    reset_terminal_viewport()?;
    let height = height.max(1);
    let terminal = Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Inline(height),
        },
    );
    if terminal.is_err() {
        RATATUI_INLINE_DISABLED.store(true, Ordering::Relaxed);
    }
    terminal
}

pub(super) fn finish_inline_terminal(mut terminal: InlineTerminal) -> io::Result<()> {
    terminal.clear()?;
    terminal.flush()
}

pub(super) fn inline_height(max_content_height: u16) -> u16 {
    terminal_height().max(1).min(max_content_height.max(1))
}

pub(super) fn inline_height_for_lines(lines: usize) -> u16 {
    inline_height(u16::try_from(lines).unwrap_or(u16::MAX))
}

pub(super) fn selected_or_focused_indices(selected: &[bool], selected_index: usize) -> Vec<usize> {
    let selected_indices = selected
        .iter()
        .enumerate()
        .filter_map(|(index, selected)| selected.then_some(index))
        .collect::<Vec<_>>();
    if selected_indices.is_empty() {
        vec![selected_index]
    } else {
        selected_indices
    }
}

fn input_render_width() -> usize {
    terminal_width().saturating_sub(1).max(1)
}

fn prompt_input_view(prompt: &str, input: &str) -> String {
    prompt_input_view_for_width(prompt, input, input_render_width())
}

fn prompt_input_view_for_width(prompt: &str, input: &str, width: usize) -> String {
    let width = width.max(1);
    let prompt_width = visible_width(prompt);
    if prompt_width >= width {
        return truncate_to_width(prompt, width);
    }

    let input_width = width - prompt_width;
    let input = input_view_for_width(input, input_width);
    format!("{prompt}{input}")
}

fn input_view_for_width(input: &str, max_width: usize) -> String {
    let input = single_line(input);
    if visible_width(&input) <= max_width {
        return input;
    }
    if max_width <= 1 {
        return ".".to_string();
    }
    format!("…{}", suffix_to_width(&input, max_width - 1))
}

fn single_line(text: &str) -> String {
    text.chars()
        .map(|ch| if ch == '\r' || ch == '\n' { ' ' } else { ch })
        .collect()
}

fn suffix_to_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    let mut suffix = Vec::new();
    let mut width = 0usize;
    for ch in text.chars().rev() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        suffix.push(ch);
        width += ch_width;
    }
    suffix.into_iter().rev().collect()
}

fn cursor_column_for(line: &str) -> u16 {
    visible_width(line).min(input_render_width()) as u16
}

fn completion_line_budget(footer: Option<&FooterLines>) -> usize {
    let rows = terminal_height();
    if rows == 0 {
        return 0;
    }

    let footer_height = if footer.is_some() && rows > FooterLines::height() {
        FooterLines::height()
    } else {
        0
    };
    usize::from(rows.saturating_sub(footer_height).saturating_sub(1))
}

fn body_line_budget_from_cursor() -> usize {
    let rows = terminal_height();
    if rows == 0 {
        return 0;
    }

    let footer_height = if rows > FooterLines::height() {
        FooterLines::height()
    } else {
        0
    };
    usize::from(rows.saturating_sub(footer_height))
}

fn fit_status_parts(left: &str, right: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let left = if visible_width(left) > width {
        truncate_to_width(left, width)
    } else {
        left.to_string()
    };
    let left_width = visible_width(&left);
    let right_width = visible_width(right);
    if left_width + 2 + right_width <= width {
        return format!(
            "{left}{}{right}",
            " ".repeat(width.saturating_sub(left_width + right_width))
        );
    }

    let available_right = width.saturating_sub(left_width + 2);
    if available_right == 0 {
        return left;
    }
    let right = truncate_to_width(right, available_right);
    format!(
        "{left}{}{right}",
        " ".repeat(width.saturating_sub(left_width + visible_width(&right)))
    )
}

fn format_context_usage(totals: &UsageTotals, context_window: Option<u64>) -> String {
    let Some(context_window) = context_window else {
        return "?/? (auto)".to_string();
    };
    if context_window == 0 {
        return "?/? (auto)".to_string();
    }

    let percent = totals.context_tokens() as f64 * 100.0 / context_window as f64;
    format!("{:.1}%/{} (auto)", percent, format_tokens(context_window))
}

fn format_tokens(count: u64) -> String {
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

fn handle_command(runtime: &mut TuiRuntime, input: &str) -> io::Result<bool> {
    refresh_active_footer(runtime);
    match parse_command(input) {
        Some(AppCommand::Quit) => {
            exit_process();
        }
        Some(AppCommand::Model) => {
            open_model_menu(runtime)?;
            Ok(true)
        }
        Some(AppCommand::Auth) => {
            open_auth_menu(runtime)?;
            Ok(true)
        }
        Some(AppCommand::Settings) => {
            settings::open_settings_menu(runtime)?;
            Ok(true)
        }
        Some(AppCommand::SettingsAuth) => {
            settings::open_auth_settings(runtime)?;
            Ok(true)
        }
        Some(AppCommand::SettingsModel) => {
            settings::open_model_settings(runtime)?;
            Ok(true)
        }
        Some(AppCommand::SettingsTheme) => {
            settings::open_theme_settings(runtime)?;
            Ok(true)
        }
        Some(AppCommand::SettingsLanguage) => {
            settings::open_language_settings(runtime)?;
            Ok(true)
        }
        Some(AppCommand::SettingsTui) => {
            settings::open_tui_settings(runtime)?;
            Ok(true)
        }
        Some(AppCommand::Debug) => {
            open_debug_menu(runtime)?;
            Ok(true)
        }
        Some(AppCommand::DebugEnable) => {
            match runtime.set_prompt_display_enabled(true) {
                Ok(()) => println!("debug prompt: enabled"),
                Err(error) => eprintln!("error: {error}"),
            }
            Ok(true)
        }
        Some(AppCommand::DebugDisable) => {
            match runtime.set_prompt_display_enabled(false) {
                Ok(()) => println!("debug prompt: disabled"),
                Err(error) => eprintln!("error: {error}"),
            }
            Ok(true)
        }
        Some(AppCommand::DebugShow) => {
            print_prompt_preview(runtime)?;
            Ok(true)
        }
        Some(AppCommand::Session) => {
            open_session_menu(runtime)?;
            Ok(true)
        }
        Some(AppCommand::Compact) => {
            match runtime.compact_context() {
                Ok(count) => println!("compacted {count} message(s)"),
                Err(error) => eprintln!("error: {error}"),
            }
            Ok(true)
        }
        Some(AppCommand::Unknown(command)) => {
            println!("unknown command: {command}");
            Ok(true)
        }
        None => Ok(false),
    }
}

fn open_debug_menu(runtime: &mut TuiRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    let labels = ["prompt"];
    let selected_index = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        select_with_keys("Debug:", &labels)?
    } else {
        println!("Debug:");
        for (index, label) in labels.iter().enumerate() {
            println!("{}) {label}", index + 1);
        }
        let Some(line) = read_cancelable_line("select debug menu: ")? else {
            println!("debug configuration cancelled");
            return Ok(());
        };
        match line.trim().parse::<usize>() {
            Ok(value) if value > 0 && value <= labels.len() => Some(value - 1),
            _ => {
                println!("invalid debug menu");
                return Ok(());
            }
        }
    };

    let Some(selected_index) = selected_index else {
        println!("debug configuration cancelled");
        return Ok(());
    };

    if selected_index == 0 {
        return open_debug_prompt_menu(runtime);
    }

    Ok(())
}

fn open_debug_prompt_menu(runtime: &mut TuiRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    let labels = ["enable", "disable"];
    let selected_index = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        select_with_keys("Debug prompt:", &labels)?
    } else {
        println!("Debug prompt:");
        for (index, label) in labels.iter().enumerate() {
            println!("{}) {label}", index + 1);
        }
        let Some(line) = read_cancelable_line("select prompt action: ")? else {
            println!("debug prompt configuration cancelled");
            return Ok(());
        };
        match line.trim().parse::<usize>() {
            Ok(value) if value > 0 && value <= labels.len() => Some(value - 1),
            _ => {
                println!("invalid prompt action");
                return Ok(());
            }
        }
    };

    let Some(selected_index) = selected_index else {
        println!("debug prompt configuration cancelled");
        return Ok(());
    };

    let enabled = selected_index == 0;
    match runtime.set_prompt_display_enabled(enabled) {
        Ok(()) => println!(
            "debug prompt: {}",
            if enabled { "enabled" } else { "disabled" }
        ),
        Err(error) => eprintln!("error: {error}"),
    }
    Ok(())
}

fn open_model_menu(runtime: &mut TuiRuntime) -> io::Result<()> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return open_model_selector(runtime);
    }

    open_model_menu_line(runtime)
}

fn open_model_selector(runtime: &mut TuiRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    let models = runtime.selectable_models();
    if models.is_empty() {
        println!("no models available");
        return Ok(());
    }

    let _raw_mode = RawModeGuard::enable()?;
    let mut selected_index = models
        .iter()
        .position(|model| model.is_current)
        .unwrap_or(0);
    let mut terminal =
        match inline_terminal(inline_height_for_lines(models.len().saturating_add(4))) {
            Ok(terminal) => terminal,
            Err(_) => {
                reset_terminal_viewport()?;
                return open_model_selector_manual_loop(runtime, models, selected_index);
            }
        };

    loop {
        terminal
            .draw(|frame| render_model_selector_view(frame, runtime, &models, selected_index))?;

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
                    models.len() - 1
                } else {
                    selected_index - 1
                };
            }
            KeyCode::Down => {
                selected_index = (selected_index + 1) % models.len();
            }
            KeyCode::Enter => {
                finish_inline_terminal(terminal)?;
                match runtime.select_model(models[selected_index].index) {
                    Ok(()) => print_fitted_terminal_line(format!(
                        "selected model: {}",
                        runtime.model_label()
                    ))?,
                    Err(error) => print_fitted_terminal_line(format!("error: {error}"))?,
                }
                return Ok(());
            }
            KeyCode::Char(' ') => {}
            KeyCode::Esc => {
                finish_inline_terminal(terminal)?;
                print_fitted_terminal_line("model selection cancelled")?;
                return Ok(());
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let _ = finish_inline_terminal(terminal);
                exit_process();
            }
            _ => {}
        }
    }
}

fn open_model_selector_manual_loop(
    runtime: &mut TuiRuntime,
    models: Vec<ModelMenuItem>,
    mut selected_index: usize,
) -> io::Result<()> {
    let _raw_mode = RawModeGuard::enable()?;
    let mut rendered_lines = 0usize;
    let mut needs_render = true;

    loop {
        if needs_render {
            rendered_lines =
                render_model_selector_manual(runtime, &models, selected_index, rendered_lines)?;
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
                    models.len() - 1
                } else {
                    selected_index - 1
                };
                needs_render = true;
            }
            KeyCode::Down => {
                selected_index = (selected_index + 1) % models.len();
                needs_render = true;
            }
            KeyCode::Enter => {
                clear_rendered_block(rendered_lines)?;
                match runtime.select_model(models[selected_index].index) {
                    Ok(()) => print_fitted_terminal_line(format!(
                        "selected model: {}",
                        runtime.model_label()
                    ))?,
                    Err(error) => print_fitted_terminal_line(format!("error: {error}"))?,
                }
                return Ok(());
            }
            KeyCode::Char(' ') => {}
            KeyCode::Esc => {
                clear_rendered_block(rendered_lines)?;
                print_fitted_terminal_line("model selection cancelled")?;
                return Ok(());
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                clear_rendered_block(rendered_lines)?;
                exit_process();
            }
            _ => {}
        }
    }
}

fn render_model_selector_manual(
    runtime: &TuiRuntime,
    models: &[ModelMenuItem],
    selected_index: usize,
    previous_lines: usize,
) -> io::Result<usize> {
    refresh_active_footer(runtime);
    if previous_lines > 0 {
        clear_rendered_block(previous_lines)?;
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "{} {}",
        bold("model"),
        dim("↑/↓ move  Space select  Enter confirm  Esc cancel  Ctrl+C exit")
    ));
    lines.push(format!("current: {}", runtime.model_label()));
    lines.push(String::new());

    for (index, model) in models.iter().enumerate() {
        let pointer = if index == selected_index { ">" } else { " " };
        let selected = index == selected_index;
        let current_marker = if model.is_current { " *" } else { "" };
        let label = format!(
            "{pointer} {}/{}  {}{}",
            model.provider, model.id, model.name, current_marker
        );
        lines.push(if selected { accent(&label) } else { label });
    }

    let rendered_lines = print_fitted_terminal_lines(&lines)?;
    render_active_footer()?;
    io::stdout().flush()?;
    Ok(rendered_lines)
}

fn render_model_selector_view(
    frame: &mut Frame<'_>,
    runtime: &TuiRuntime,
    models: &[ModelMenuItem],
    selected_index: usize,
) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    let current_height = if area.height >= 2 { 1 } else { 0 };
    let hint_height = if area.height >= 4 { 1 } else { 0 };
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(current_height),
            Constraint::Min(0),
            Constraint::Length(hint_height),
        ])
        .split(area);

    Paragraph::new(Line::from(Span::styled(
        "model",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )))
    .render(areas[0], frame.buffer_mut());

    if current_height > 0 {
        Paragraph::new(Line::from(Span::styled(
            format!("current: {}", runtime.model_label()),
            Style::default().fg(Color::DarkGray),
        )))
        .render(areas[1], frame.buffer_mut());
    }

    let body_height = usize::from(areas[2].height);
    let selected_index = selected_index.min(models.len().saturating_sub(1));
    let start = selected_index.saturating_sub(body_height.saturating_sub(1));
    let lines = models
        .iter()
        .enumerate()
        .skip(start)
        .take(body_height)
        .map(|(index, model)| {
            let selected = index == selected_index;
            let marker = if selected { "> " } else { "  " };
            let style = if selected {
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let mut spans = vec![
                Span::styled(marker, style),
                Span::styled(format!("{}/{}", model.provider, model.id), style),
                Span::raw("  "),
                Span::styled(model.name.clone(), style),
            ];
            if model.is_current {
                spans.push(Span::styled(" *", Style::default().fg(Color::Green)));
            }
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    Paragraph::new(Text::from(lines)).render(areas[2], frame.buffer_mut());

    if hint_height > 0 {
        Paragraph::new(Line::from(Span::styled(
            "↑/↓ move   enter confirm   escape cancel   ctrl+c exit",
            Style::default().fg(Color::DarkGray),
        )))
        .render(areas[3], frame.buffer_mut());
    }
}

pub(super) fn clear_rendered_block(lines: usize) -> io::Result<()> {
    if lines == 0 {
        return Ok(());
    }

    let mut stdout = io::stdout();
    execute!(
        stdout,
        cursor::MoveUp(lines as u16),
        cursor::MoveToColumn(0),
        terminal::Clear(ClearType::FromCursorDown)
    )?;
    render_active_footer()?;
    stdout.flush()
}

pub(super) struct RawModeGuard;

impl RawModeGuard {
    pub(super) fn enable() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnableBracketedPaste)?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        let _ = terminal::disable_raw_mode();
    }
}

pub(super) fn exit_process() -> ! {
    reset_scroll_region();
    let _ = terminal::disable_raw_mode();
    println!("bye");
    std::process::exit(0);
}

pub(super) fn read_cancelable_line(prompt: &str) -> io::Result<Option<String>> {
    let footer = active_footer();
    read_line_with_history(prompt, None, footer.as_ref())
}

fn read_prompt_line(
    prompt: &str,
    history: &[String],
    footer: Option<&FooterLines>,
) -> io::Result<Option<String>> {
    read_line_with_history(prompt, Some(history), footer)
}

fn read_line_with_history(
    prompt: &str,
    history: Option<&[String]>,
    footer: Option<&FooterLines>,
) -> io::Result<Option<String>> {
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        print!("{prompt}");
        io::stdout().flush()?;
        let mut line = String::new();
        let bytes = io::stdin().read_line(&mut line)?;
        if bytes == 0 {
            return Ok(None);
        }
        return Ok(Some(line));
    }

    let _raw_mode = RawModeGuard::enable()?;
    read_line_with_history_manual_loop(prompt, history, footer)
}

fn read_line_with_history_manual_loop(
    prompt: &str,
    history: Option<&[String]>,
    footer: Option<&FooterLines>,
) -> io::Result<Option<String>> {
    let mut input = String::new();
    let mut history_index = None;
    let mut draft = String::new();
    let mut completion_lines = 0usize;
    let mut paste_burst = PasteBurst::default();
    let project_dir = std::env::current_dir().unwrap_or_else(|_| ".".into());

    print!("{prompt}");
    render_footer(footer)?;
    io::stdout().flush()?;

    loop {
        if flush_plain_paste_burst_if_due(
            &mut paste_burst,
            &mut input,
            prompt,
            &mut completion_lines,
            footer,
            &project_dir,
            Instant::now(),
        )? {
            history_index = None;
        }

        let poll_timeout = paste_burst
            .next_flush_delay(Instant::now())
            .unwrap_or(Duration::from_millis(250));
        if !event::poll(poll_timeout)? {
            if flush_plain_paste_burst_if_due(
                &mut paste_burst,
                &mut input,
                prompt,
                &mut completion_lines,
                footer,
                &project_dir,
                Instant::now(),
            )? {
                history_index = None;
            }
            continue;
        }

        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                match paste_burst.observe_key(&key, Instant::now()) {
                    PasteBurstAction::Handled => continue,
                    PasteBurstAction::InsertNewline => {
                        history_index = None;
                        input.push('\n');
                        redraw_input_with_completion(
                            prompt,
                            &input,
                            &mut completion_lines,
                            footer,
                        )?;
                        continue;
                    }
                    PasteBurstAction::None => {
                        if apply_plain_paste_burst_flush(
                            paste_burst.flush_before_modified_input(),
                            &mut input,
                            prompt,
                            &mut completion_lines,
                            footer,
                            &project_dir,
                        )? {
                            history_index = None;
                        }
                    }
                }

                match key.code {
                    KeyCode::Enter => {
                        clear_completion_block(prompt, &input, &mut completion_lines, footer)?;
                        print_terminal_newline()?;
                        return Ok(Some(input));
                    }
                    KeyCode::Esc => {
                        clear_completion_block(prompt, &input, &mut completion_lines, footer)?;
                        print_terminal_newline()?;
                        return Ok(None);
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        clear_completion_block(prompt, &input, &mut completion_lines, footer)?;
                        print_terminal_newline()?;
                        exit_process();
                    }
                    KeyCode::Backspace if input.pop().is_some() => {
                        history_index = None;
                        redraw_input_with_completion(
                            prompt,
                            &input,
                            &mut completion_lines,
                            footer,
                        )?;
                    }
                    KeyCode::Tab => {
                        if let Some(completed) = complete_slash_command(&input) {
                            history_index = None;
                            input = completed;
                            redraw_input_with_completion(
                                prompt,
                                &input,
                                &mut completion_lines,
                                footer,
                            )?;
                        }
                    }
                    KeyCode::Up => {
                        let Some(history) = history else {
                            continue;
                        };
                        if history.is_empty() {
                            continue;
                        }
                        let index = match history_index {
                            Some(index) if index > 0 => index - 1,
                            Some(index) => index,
                            None => {
                                draft = input.clone();
                                history.len() - 1
                            }
                        };
                        history_index = Some(index);
                        input = history[index].clone();
                        redraw_input_with_completion(
                            prompt,
                            &input,
                            &mut completion_lines,
                            footer,
                        )?;
                    }
                    KeyCode::Down => {
                        let Some(history) = history else {
                            continue;
                        };
                        let Some(index) = history_index else {
                            continue;
                        };
                        if index + 1 < history.len() {
                            let next_index = index + 1;
                            history_index = Some(next_index);
                            input = history[next_index].clone();
                        } else {
                            history_index = None;
                            input = draft.clone();
                        }
                        redraw_input_with_completion(
                            prompt,
                            &input,
                            &mut completion_lines,
                            footer,
                        )?;
                    }
                    KeyCode::Char(value) => {
                        history_index = None;
                        input.push(value);
                        redraw_input_with_completion(
                            prompt,
                            &input,
                            &mut completion_lines,
                            footer,
                        )?;
                    }
                    _ => {}
                }
            }
            Event::Paste(value) => {
                paste_burst.mark_bracketed_paste_seen();
                history_index = None;
                match prepare_paste_text(&project_dir, &value) {
                    Ok(prepared) => input.push_str(&prepared),
                    Err(_) => input.push_str(&truncate_large_paste_fallback(&value)),
                }
                redraw_input_with_completion(prompt, &input, &mut completion_lines, footer)?;
            }
            Event::Resize(_, _) => {
                reset_terminal_viewport()?;
            }
            _ => {}
        }
    }
}

fn flush_plain_paste_burst_if_due(
    paste_burst: &mut PasteBurst,
    input: &mut String,
    prompt: &str,
    completion_lines: &mut usize,
    footer: Option<&FooterLines>,
    project_dir: &std::path::Path,
    now: Instant,
) -> io::Result<bool> {
    apply_plain_paste_burst_flush(
        paste_burst.flush_if_due(now),
        input,
        prompt,
        completion_lines,
        footer,
        project_dir,
    )
}

fn apply_plain_paste_burst_flush(
    result: FlushResult,
    input: &mut String,
    prompt: &str,
    completion_lines: &mut usize,
    footer: Option<&FooterLines>,
    project_dir: &std::path::Path,
) -> io::Result<bool> {
    match result {
        FlushResult::Paste(text) => match prepare_paste_text(project_dir, &text) {
            Ok(prepared) => input.push_str(&prepared),
            Err(_) => input.push_str(&truncate_large_paste_fallback(&text)),
        },
        FlushResult::Typed(ch) => input.push(ch),
        FlushResult::None => return Ok(false),
    }
    redraw_input_with_completion(prompt, input, completion_lines, footer)?;
    Ok(true)
}

fn redraw_input_with_completion(
    prompt: &str,
    input: &str,
    completion_lines: &mut usize,
    footer: Option<&FooterLines>,
) -> io::Result<()> {
    let prompt_line = prompt_input_view(prompt, input);
    let mut stdout = io::stdout();
    execute!(
        stdout,
        cursor::MoveToColumn(0),
        terminal::Clear(ClearType::FromCursorDown)
    )?;
    print!("{prompt_line}");

    let suggestions = slash_command_suggestions(input);
    let line_budget = completion_line_budget(footer);
    *completion_lines = suggestions.len().min(line_budget);
    for (index, suggestion) in suggestions.into_iter().take(*completion_lines).enumerate() {
        let command = suggestion.command.trim_start_matches('/');
        if index == 0 {
            print_terminal_line("")?;
        }
        let command = format!("{command:<18}");
        print_fitted_terminal_line(format!(
            "  {} {}",
            accent(&command),
            dim(tr(Locale::En, suggestion.description_id))
        ))?;
    }

    if *completion_lines > 0 {
        execute!(
            stdout,
            cursor::MoveUp((*completion_lines + 1) as u16),
            cursor::MoveToColumn(cursor_column_for(&prompt_line))
        )?;
    }
    render_footer(footer)?;
    stdout.flush()
}

pub(super) fn redraw_current_prompt_line(prompt: &str, input: &str) -> io::Result<()> {
    let prompt_line = prompt_input_view(prompt, input);
    let mut stdout = io::stdout();
    execute!(
        stdout,
        cursor::MoveToColumn(0),
        terminal::Clear(ClearType::CurrentLine)
    )?;
    print!("{prompt_line}");
    execute!(
        stdout,
        cursor::MoveToColumn(cursor_column_for(&prompt_line))
    )?;
    stdout.flush()
}

fn clear_completion_block(
    prompt: &str,
    input: &str,
    completion_lines: &mut usize,
    footer: Option<&FooterLines>,
) -> io::Result<()> {
    if *completion_lines == 0 {
        render_footer(footer)?;
        return Ok(());
    }
    let mut stdout = io::stdout();
    execute!(
        stdout,
        cursor::MoveDown(1),
        cursor::MoveToColumn(0),
        terminal::Clear(ClearType::FromCursorDown),
        cursor::MoveUp(1),
        cursor::MoveToColumn(cursor_column_for(&prompt_input_view(prompt, input)))
    )?;
    *completion_lines = 0;
    render_footer(footer)?;
    stdout.flush()
}

fn render_footer(footer: Option<&FooterLines>) -> io::Result<()> {
    let Some(footer) = footer else {
        return Ok(());
    };
    enable_footer_scroll_region()?;
    let rows = terminal_height();
    let footer_height = FooterLines::height();
    if rows <= footer_height {
        return Ok(());
    }

    let mut stdout = io::stdout();
    execute!(
        stdout,
        cursor::SavePosition,
        cursor::MoveTo(0, rows.saturating_sub(footer_height)),
        terminal::Clear(ClearType::CurrentLine),
        cursor::MoveTo(0, rows.saturating_sub(footer_height - 1)),
        terminal::Clear(ClearType::CurrentLine)
    )?;
    print!(
        "{}",
        dim(&truncate_to_width(&footer.path, terminal_width()))
    );
    execute!(
        stdout,
        cursor::MoveTo(0, rows.saturating_sub(1)),
        terminal::Clear(ClearType::CurrentLine)
    )?;
    print!(
        "{}",
        dim(&truncate_to_width(&footer.status, terminal_width()))
    );
    execute!(stdout, cursor::RestorePosition)?;
    stdout.flush()
}

fn enable_footer_scroll_region() -> io::Result<()> {
    let rows = terminal_height();
    let footer_height = FooterLines::height();
    if rows <= footer_height {
        return Ok(());
    }

    let mut stdout = io::stdout();
    execute!(stdout, cursor::SavePosition)?;
    write!(
        stdout,
        "{TERMINAL_VIEWPORT_RESET}\x1b[1;{}r",
        rows.saturating_sub(footer_height)
    )?;
    execute!(stdout, cursor::RestorePosition)?;
    stdout.flush()
}

pub(super) fn reset_terminal_viewport() -> io::Result<()> {
    let mut stdout = io::stdout();
    execute!(stdout, cursor::SavePosition)?;
    write!(stdout, "{TERMINAL_VIEWPORT_RESET}")?;
    execute!(stdout, cursor::RestorePosition)?;
    stdout.flush()
}

fn reset_scroll_region() {
    let mut stdout = io::stdout();
    let _ = execute!(stdout, cursor::SavePosition);
    let _ = write!(stdout, "{TERMINAL_VIEWPORT_RESET}");
    let _ = execute!(stdout, cursor::RestorePosition);
    let _ = stdout.flush();
}

fn slash_command_suggestions(input: &str) -> Vec<crate::commands::CommandHelp> {
    if !input.starts_with('/') {
        return Vec::new();
    }
    COMMAND_HELP
        .iter()
        .copied()
        .filter(|entry| entry.command.starts_with(input))
        .filter(|entry| !entry.command.trim_start_matches('/').contains(' '))
        .collect()
}

fn complete_slash_command(input: &str) -> Option<String> {
    if !input.starts_with('/') {
        return None;
    }
    let matches = slash_command_suggestions(input);
    if matches.is_empty() {
        return None;
    }
    if matches.len() == 1 {
        return Some(format!("{} ", matches[0].command));
    }

    let common = common_prefix(matches.iter().map(|entry| entry.command));
    if common.len() > input.len() {
        Some(common)
    } else {
        Some(matches[0].command.to_string())
    }
}

fn common_prefix<'a>(values: impl Iterator<Item = &'a str>) -> String {
    let mut values = values;
    let Some(first) = values.next() else {
        return String::new();
    };
    let mut prefix = first.to_string();
    for value in values {
        while !value.starts_with(&prefix) {
            if prefix.is_empty() {
                return prefix;
            }
            prefix.pop();
        }
    }
    prefix
}

fn open_model_menu_line(runtime: &mut TuiRuntime) -> io::Result<()> {
    println!("current model: {}", runtime.model_label());
    println!();

    let models = runtime.selectable_models();
    if models.is_empty() {
        println!("no models available");
        return Ok(());
    }

    for (index, model) in models.iter().enumerate() {
        let marker = if model.is_current { "*" } else { " " };
        println!("{}) [{}] {}", index + 1, marker, model.name);
    }

    let Some(line) = read_cancelable_line("select model: ")? else {
        println!("model selection cancelled");
        return Ok(());
    };
    let choice = line.trim();
    if choice.is_empty() {
        return Ok(());
    }

    let selected_index = match choice.parse::<usize>() {
        Ok(value) if value > 0 && value <= models.len() => value - 1,
        _ => {
            println!("invalid model selection");
            return Ok(());
        }
    };

    match runtime.select_model(models[selected_index].index) {
        Ok(()) => println!("selected model: {}", runtime.model_label()),
        Err(error) => eprintln!("error: {error}"),
    }

    Ok(())
}

fn open_session_menu(runtime: &mut TuiRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    println!("current session: {}", runtime.session_id());
    println!("path: {}", runtime.session_path());
    println!("messages: {}", runtime.session_message_count());
    println!();

    let current_session_id = runtime.session_id().to_string();
    let mut sessions = match runtime.list_sessions() {
        Ok(sessions) => sessions,
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(());
        }
    };
    sessions.retain(|session| session.id != current_session_id);

    println!("0) new session");
    for (index, session) in sessions.iter().enumerate() {
        println!(
            "{}) {}  messages={}  cwd={}{}",
            index + 1,
            session.id,
            session.message_count,
            session.cwd,
            session
                .preview
                .as_deref()
                .map(|preview| format!("  preview={preview}"))
                .unwrap_or_default()
        );
    }
    let Some(line) = read_cancelable_line("select session: ")? else {
        println!("session selection cancelled");
        return Ok(());
    };
    let choice = line.trim();
    if choice.is_empty() {
        return Ok(());
    }

    if choice == "0" {
        match runtime.start_new_session() {
            Ok(()) => {
                refresh_active_footer(runtime);
                println!("new session: {}", runtime.session_id());
            }
            Err(error) => eprintln!("error: {error}"),
        }
        return Ok(());
    }

    let selected = match choice.parse::<usize>() {
        Ok(value) if value > 0 && value <= sessions.len() => &sessions[value - 1],
        _ => {
            println!("invalid session selection");
            return Ok(());
        }
    };

    match runtime.open_session(&selected.path) {
        Ok(()) => {
            refresh_active_footer(runtime);
            println!("loaded session: {}", runtime.session_id());
            print_recent_messages(runtime);
        }
        Err(error) => eprintln!("error: {error}"),
    }

    Ok(())
}

fn print_recent_messages(runtime: &TuiRuntime) {
    let messages = runtime.recent_messages(6);
    if messages.is_empty() {
        return;
    }

    println!("recent messages:");
    for message in messages {
        println!(
            "- {}: {}",
            message.role,
            truncate_single_line(&message.content, 120)
        );
    }
}

fn truncate_single_line(content: &str, max_chars: usize) -> String {
    let mut text = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max_chars {
        return text;
    }

    text = text.chars().take(max_chars.saturating_sub(3)).collect();
    text.push_str("...");
    text
}

struct ThinkingSpinner {
    active: Option<Arc<AtomicBool>>,
    handle: Option<JoinHandle<()>>,
}

impl ThinkingSpinner {
    fn start(footer: Option<FooterLines>) -> Self {
        if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
            return Self {
                active: None,
                handle: None,
            };
        }

        let active = Arc::new(AtomicBool::new(true));
        let thread_active = Arc::clone(&active);
        println!();
        let _ = io::stdout().flush();
        let handle = thread::spawn(move || {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut index = 0usize;
            while thread_active.load(Ordering::Relaxed) {
                print!(
                    "\r{} {}",
                    cyan(frames[index % frames.len()]),
                    dim("Thinking...")
                );
                let _ = render_footer(footer.as_ref());
                let _ = io::stdout().flush();
                index += 1;
                thread::sleep(Duration::from_millis(120));
            }
        });

        Self {
            active: Some(active),
            handle: Some(handle),
        }
    }

    fn stop(&mut self) {
        if let Some(active) = self.active.take() {
            active.store(false, Ordering::Relaxed);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
            print!("\r\x1b[2K");
            let _ = io::stdout().flush();
        }
    }
}

impl Drop for ThinkingSpinner {
    fn drop(&mut self) {
        self.stop();
    }
}

fn current_directory_label(runtime: &TuiRuntime) -> String {
    runtime
        .project_dir()
        .display()
        .to_string()
        .replace('\\', "/")
}

fn visible_width(text: &str) -> usize {
    let mut width = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for ch in chars.by_ref() {
                if ch.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        width += UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    width
}

pub(super) fn strip_ansi(text: &str) -> String {
    let mut plain = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for ch in chars.by_ref() {
                if ch.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        plain.push(ch);
    }
    plain
}

fn clear_screen() {
    print!("{TERMINAL_VIEWPORT_RESET}\x1b[2J\x1b[H");
}

pub(super) fn bold(text: &str) -> String {
    format!("\x1b[1m{text}\x1b[0m")
}

pub(super) fn dim(text: &str) -> String {
    format!("\x1b[2m{text}\x1b[0m")
}

pub(super) fn thinking(text: &str) -> String {
    format!("\x1b[2;3m{text}\x1b[0m")
}

pub(super) fn accent(text: &str) -> String {
    format!("\x1b[35;1m{text}\x1b[0m")
}

fn cyan(text: &str) -> String {
    format!("\x1b[36;1m{text}\x1b[0m")
}

#[cfg(test)]
mod tests {
    use super::*;
    use exgent_core::ModelSettingsItem;

    #[test]
    fn ansi_truncation_respects_visible_width() {
        let truncated = truncate_to_width(&accent("abcdefghijklmnopqrstuvwxyz"), 10);

        assert!(visible_width(&truncated) <= 10);
        assert!(truncated.ends_with("\x1b[0m"));
    }

    #[test]
    fn prompt_input_view_does_not_exceed_terminal_width() {
        let input = "x".repeat(200);
        let rendered = prompt_input_view(&format!("{} ", bold(">")), &input);

        assert!(visible_width(&rendered) <= input_render_width());
    }

    #[test]
    fn suffix_to_width_handles_wide_characters() {
        let suffix = suffix_to_width("abc你好", 4);

        assert!(visible_width(&suffix) <= 4);
        assert_eq!(suffix, "你好");
    }

    #[test]
    fn selected_or_focused_indices_defaults_to_focused_item() {
        assert_eq!(
            selected_or_focused_indices(&[false, false, false], 1),
            vec![1]
        );
        assert_eq!(
            selected_or_focused_indices(&[true, false, true], 1),
            vec![0, 2]
        );
    }

    #[test]
    fn slash_suggestions_only_show_top_level_commands() {
        let suggestions = slash_command_suggestions("/");
        let commands = suggestions
            .iter()
            .map(|entry| entry.command)
            .collect::<Vec<_>>();

        assert!(commands.contains(&"/debug"));
        assert!(commands.contains(&"/settings"));
        assert!(!commands.contains(&"/debug show"));
        assert!(!commands.contains(&"/settings auth"));
        assert!(slash_command_suggestions("/debug ").is_empty());
    }

    #[test]
    fn model_settings_rows_tracks_selected_row_after_provider_headers() {
        let models = vec![
            ModelSettingsItem {
                index: 0,
                provider: "alpha".to_string(),
                id: "alpha-one".to_string(),
                name: "Alpha One".to_string(),
                adapter: "openai".to_string(),
                is_enabled: true,
                is_custom: false,
            },
            ModelSettingsItem {
                index: 1,
                provider: "beta".to_string(),
                id: "beta-one".to_string(),
                name: "Beta One".to_string(),
                adapter: "openai".to_string(),
                is_enabled: true,
                is_custom: false,
            },
        ];

        let (_rows, selected_row) =
            super::settings::model_settings_rows(&models, &[false, true], 1);

        assert_eq!(selected_row, 3);
    }
}
