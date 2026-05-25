use std::{
    env,
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
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, ClearType},
};
use exgent_core::AgentEvent;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Widget},
    Frame, Terminal, TerminalOptions, Viewport,
};
use unicode_width::UnicodeWidthChar;

use crate::{
    app::{AppRuntime, ModelMenuItem, ModelSettingsItem, UsageTotals},
    commands::{parse_command, AppCommand, COMMAND_HELP},
    localization::{tr, Locale, LANGUAGE_OPTIONS},
    oauth::{
        finish_anthropic_oauth_flow, finish_github_copilot_device_flow, normalize_github_domain,
        parse_authorization_input, start_anthropic_oauth_flow, start_github_copilot_device_flow,
        AnthropicOAuthFlow, AuthorizationCode,
    },
    settings::{ThemeRgb, ThemeSettings, THEME_PRESETS},
};

mod fullscreen;

static ACTIVE_FOOTER: OnceLock<Mutex<Option<FooterLines>>> = OnceLock::new();
static RATATUI_INLINE_DISABLED: AtomicBool = AtomicBool::new(false);
const TERMINAL_VIEWPORT_RESET: &str = "\x1b[r\x1b[?6l";
type InlineTerminal = Terminal<CrosstermBackend<io::Stdout>>;

pub fn run(runtime: &mut AppRuntime) -> io::Result<()> {
    if io::stdin().is_terminal()
        && io::stdout().is_terminal()
        && env::var_os("EXGENT_LEGACY_TUI").is_none()
    {
        return fullscreen::run(runtime);
    }

    run_legacy(runtime)
}

fn run_legacy(runtime: &mut AppRuntime) -> io::Result<()> {
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

        let show_prompt_details = runtime.prompt_display_enabled();
        let mut renderer = EventRenderer::new(
            show_prompt_details,
            io::stdin().is_terminal() && io::stdout().is_terminal(),
        );
        let spinner_footer = (io::stdin().is_terminal() && io::stdout().is_terminal())
            .then(|| footer_lines(runtime, terminal_width()));
        let mut spinner = ThinkingSpinner::start(spinner_footer);
        match runtime.run_prompt_streaming(input, &mut |event| {
            if event_has_visible_output(&event, show_prompt_details) {
                spinner.stop();
            }
            renderer.render(event);
        }) {
            Ok(()) => {}
            Err(error) => {
                spinner.stop();
                eprintln!("error: {error}");
            }
        }
        spinner.stop();
    }

    Ok(())
}

fn print_startup(runtime: &AppRuntime) -> io::Result<()> {
    clear_screen();

    let directory = current_directory_label();
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

fn print_prompt_preview(runtime: &AppRuntime) -> io::Result<()> {
    println!("{}", dim("system prompt:"));
    println!("{}", runtime.system_prompt());
    println!("{}", dim("end system prompt"));
    io::stdout().flush()
}

fn status_line(runtime: &AppRuntime, width: usize) -> String {
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

fn footer_lines(runtime: &AppRuntime, width: usize) -> FooterLines {
    FooterLines {
        path: truncate_to_width(&current_directory_label(), width),
        status: status_line(runtime, width),
    }
}

fn refresh_active_footer(runtime: &AppRuntime) {
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

fn render_active_footer() -> io::Result<()> {
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

fn print_fitted_terminal_line(line: impl AsRef<str>) -> io::Result<()> {
    print_terminal_line(truncate_to_width(line.as_ref(), terminal_width()))
}

fn print_fitted_terminal_lines(lines: &[String]) -> io::Result<usize> {
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

fn print_terminal_newline() -> io::Result<()> {
    let mut stdout = io::stdout();
    write!(stdout, "\r\n")?;
    stdout.flush()
}

fn inline_terminal(height: u16) -> io::Result<InlineTerminal> {
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

fn finish_inline_terminal(mut terminal: InlineTerminal) -> io::Result<()> {
    terminal.clear()?;
    terminal.flush()
}

fn inline_height(max_content_height: u16) -> u16 {
    terminal_height().max(1).min(max_content_height.max(1))
}

fn inline_height_for_lines(lines: usize) -> u16 {
    inline_height(u16::try_from(lines).unwrap_or(u16::MAX))
}

fn selected_or_focused_indices(selected: &[bool], selected_index: usize) -> Vec<usize> {
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

fn handle_command(runtime: &mut AppRuntime, input: &str) -> io::Result<bool> {
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
            open_settings_menu(runtime)?;
            Ok(true)
        }
        Some(AppCommand::SettingsAuth) => {
            open_auth_settings(runtime)?;
            Ok(true)
        }
        Some(AppCommand::SettingsModel) => {
            open_model_settings(runtime)?;
            Ok(true)
        }
        Some(AppCommand::SettingsTheme) => {
            open_theme_settings(runtime)?;
            Ok(true)
        }
        Some(AppCommand::SettingsLanguage) => {
            open_language_settings(runtime)?;
            Ok(true)
        }
        Some(AppCommand::Debug) => {
            open_debug_menu(runtime)?;
            Ok(true)
        }
        Some(AppCommand::DebugEnable) => {
            match runtime.set_prompt_display_enabled(true) {
                Ok(()) => println!("debug display: enabled"),
                Err(error) => eprintln!("error: {error}"),
            }
            Ok(true)
        }
        Some(AppCommand::DebugDisable) => {
            match runtime.set_prompt_display_enabled(false) {
                Ok(()) => println!("debug display: disabled"),
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

fn open_debug_menu(runtime: &mut AppRuntime) -> io::Result<()> {
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

fn open_debug_prompt_menu(runtime: &mut AppRuntime) -> io::Result<()> {
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

fn open_model_menu(runtime: &mut AppRuntime) -> io::Result<()> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return open_model_selector(runtime);
    }

    open_model_menu_line(runtime)
}

fn open_model_selector(runtime: &mut AppRuntime) -> io::Result<()> {
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
    runtime: &mut AppRuntime,
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
    runtime: &AppRuntime,
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
    runtime: &AppRuntime,
    models: &[ModelMenuItem],
    selected_index: usize,
) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    let current_height = (area.height >= 2).then_some(1).unwrap_or(0);
    let hint_height = (area.height >= 4).then_some(1).unwrap_or(0);
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

fn open_model_settings(runtime: &mut AppRuntime) -> io::Result<()> {
    let models = enabled_provider_model_settings_items(&runtime.model_settings_items());
    if models.is_empty() {
        println!("no configured models available");
        println!("use /settings auth to enable a provider first");
        return Ok(());
    }

    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        edit_model_settings_with_keys(runtime, models)
    } else {
        edit_model_settings_line(runtime, &models)
    }
}

fn enabled_provider_model_settings_items(models: &[ModelSettingsItem]) -> Vec<ModelSettingsItem> {
    models
        .iter()
        .filter(|model| {
            models
                .iter()
                .any(|candidate| candidate.provider == model.provider && candidate.is_enabled)
        })
        .cloned()
        .collect()
}

fn open_settings_menu(runtime: &mut AppRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    let labels = ["auth", "model", "theme", "language"];
    let selected = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        select_with_keys("Select settings:", &labels)?
    } else {
        println!("Select settings:");
        println!("1) auth");
        println!("2) model");
        println!("3) theme");
        println!("4) language");
        let Some(line) = read_cancelable_line("select settings: ")? else {
            return Ok(());
        };
        match line.trim() {
            "1" | "auth" => Some(0),
            "2" | "model" => Some(1),
            "3" | "theme" => Some(2),
            "4" | "language" => Some(3),
            "" => None,
            _ => {
                println!("invalid settings selection");
                None
            }
        }
    };

    match selected {
        Some(0) => open_auth_settings(runtime),
        Some(1) => open_model_settings(runtime),
        Some(2) => open_theme_settings(runtime),
        Some(3) => open_language_settings(runtime),
        _ => {
            println!("settings cancelled");
            Ok(())
        }
    }
}

fn open_theme_settings(runtime: &mut AppRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    let current = runtime.theme();
    let mut labels = THEME_PRESETS
        .iter()
        .map(|preset| {
            let marker = if current.name == preset.name && current.rgb == preset.rgb {
                " *"
            } else {
                ""
            };
            format!(
                "{}  rgb({}, {}, {}){}",
                preset.name, preset.rgb.r, preset.rgb.g, preset.rgb.b, marker
            )
        })
        .collect::<Vec<_>>();
    labels.push("custom rgb".to_string());
    let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();

    let selected = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        select_with_keys("Select theme:", &label_refs)?
    } else {
        println!("Select theme:");
        for (index, label) in labels.iter().enumerate() {
            println!("{}) {label}", index + 1);
        }
        let Some(line) = read_cancelable_line("select theme: ")? else {
            return Ok(());
        };
        match line.trim().parse::<usize>() {
            Ok(value) if value > 0 && value <= labels.len() => Some(value - 1),
            _ => None,
        }
    };

    let Some(selected) = selected else {
        println!("theme cancelled");
        return Ok(());
    };

    if let Some(preset) = THEME_PRESETS.get(selected).copied() {
        match runtime.set_theme(ThemeSettings::preset(preset)) {
            Ok(()) => println!("theme: {}", preset.name),
            Err(error) => eprintln!("error: {error}"),
        }
        return Ok(());
    }

    let Some(red) = read_rgb_component("red")? else {
        println!("theme cancelled");
        return Ok(());
    };
    let Some(green) = read_rgb_component("green")? else {
        println!("theme cancelled");
        return Ok(());
    };
    let Some(blue) = read_rgb_component("blue")? else {
        println!("theme cancelled");
        return Ok(());
    };

    let rgb = ThemeRgb::new(red, green, blue);
    match runtime.set_theme(ThemeSettings::custom(rgb)) {
        Ok(()) => println!("theme: custom rgb({red}, {green}, {blue})"),
        Err(error) => eprintln!("error: {error}"),
    }
    Ok(())
}

fn open_language_settings(runtime: &mut AppRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    let current = runtime.locale();
    let labels = LANGUAGE_OPTIONS
        .iter()
        .map(|option| {
            let marker = if current == option.locale { " *" } else { "" };
            format!("{}{}", option.label, marker)
        })
        .collect::<Vec<_>>();
    let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();

    let selected = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        select_with_keys("Select language:", &label_refs)?
    } else {
        println!("Select language:");
        for (index, label) in labels.iter().enumerate() {
            println!("{}) {label}", index + 1);
        }
        let Some(line) = read_cancelable_line("select language: ")? else {
            return Ok(());
        };
        match line.trim().parse::<usize>() {
            Ok(value) if value > 0 && value <= labels.len() => Some(value - 1),
            _ => None,
        }
    };

    let Some(selected) = selected else {
        println!("language cancelled");
        return Ok(());
    };

    let option = LANGUAGE_OPTIONS[selected];
    match runtime.set_locale(option.setting) {
        Ok(()) => println!("language: {}", option.locale.display_name()),
        Err(error) => eprintln!("error: {error}"),
    }
    Ok(())
}

fn read_rgb_component(label: &str) -> io::Result<Option<u8>> {
    loop {
        let Some(line) = read_cancelable_line(&format!("{label} 0-255: "))? else {
            return Ok(None);
        };
        let value = line.trim();
        if value.is_empty() {
            return Ok(None);
        }
        match value.parse::<u8>() {
            Ok(component) => return Ok(Some(component)),
            Err(_) => println!("invalid {label}: expected 0-255"),
        }
    }
}

fn open_auth_settings(runtime: &mut AppRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    let models = runtime.model_settings_items();
    if models.is_empty() {
        println!("no configured models available");
        println!("use /auth to configure a provider first");
        return Ok(());
    }
    let providers = auth_provider_settings_items(&models);

    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        open_auth_settings_with_keys(runtime, providers)
    } else {
        open_auth_settings_line(runtime, &providers)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AuthProviderSettingsItem {
    provider: String,
    model_indices: Vec<usize>,
    is_enabled: bool,
    has_built_in: bool,
}

fn auth_provider_settings_items(models: &[ModelSettingsItem]) -> Vec<AuthProviderSettingsItem> {
    let mut providers = Vec::<AuthProviderSettingsItem>::new();
    for model in models {
        if let Some(provider) = providers
            .iter_mut()
            .find(|provider| provider.provider == model.provider)
        {
            provider.model_indices.push(model.index);
            provider.is_enabled |= model.is_enabled;
            provider.has_built_in |= !model.is_custom;
        } else {
            providers.push(AuthProviderSettingsItem {
                provider: model.provider.clone(),
                model_indices: vec![model.index],
                is_enabled: model.is_enabled,
                has_built_in: !model.is_custom,
            });
        }
    }
    providers
}

fn open_auth_settings_with_keys(
    runtime: &mut AppRuntime,
    mut providers: Vec<AuthProviderSettingsItem>,
) -> io::Result<()> {
    refresh_active_footer(runtime);
    let mut selected_index = 0usize;
    let mut selected = vec![false; providers.len()];
    let mut raw_mode = RawModeGuard::enable()?;
    let mut terminal =
        match inline_terminal(inline_height_for_lines(providers.len().saturating_add(3))) {
            Ok(terminal) => terminal,
            Err(_) => {
                reset_terminal_viewport()?;
                return open_auth_settings_manual_loop(
                    runtime,
                    providers,
                    selected_index,
                    selected,
                );
            }
        };

    loop {
        terminal.draw(|frame| {
            render_auth_settings_selector_view(frame, &providers, &selected, selected_index)
        })?;

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
                    providers.len() - 1
                } else {
                    selected_index - 1
                };
            }
            KeyCode::Down => {
                selected_index = (selected_index + 1) % providers.len();
            }
            KeyCode::Char(' ') => {
                selected[selected_index] = !selected[selected_index];
            }
            KeyCode::Enter => {
                finish_inline_terminal(terminal)?;
                drop(raw_mode);
                let target_indices = selected_or_focused_indices(&selected, selected_index);
                handle_auth_provider_action(runtime, &providers, &target_indices)?;
                refresh_active_footer(runtime);
                providers = auth_provider_settings_items(&runtime.model_settings_items());
                if providers.is_empty() {
                    return Ok(());
                }
                selected = vec![false; providers.len()];
                selected_index = selected_index.min(providers.len() - 1);
                raw_mode = RawModeGuard::enable()?;
                terminal = match inline_terminal(inline_height_for_lines(
                    providers.len().saturating_add(3),
                )) {
                    Ok(terminal) => terminal,
                    Err(_) => {
                        reset_terminal_viewport()?;
                        return open_auth_settings_manual_loop(
                            runtime,
                            providers,
                            selected_index,
                            selected,
                        );
                    }
                };
            }
            KeyCode::Esc => {
                finish_inline_terminal(terminal)?;
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

fn open_auth_settings_manual_loop(
    runtime: &mut AppRuntime,
    mut providers: Vec<AuthProviderSettingsItem>,
    mut selected_index: usize,
    mut selected: Vec<bool>,
) -> io::Result<()> {
    let mut raw_mode = RawModeGuard::enable()?;
    let mut rendered_lines = 0usize;
    let mut needs_render = true;

    loop {
        if needs_render {
            rendered_lines = render_auth_settings_selector_manual(
                &providers,
                &selected,
                selected_index,
                rendered_lines,
            )?;
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
                    providers.len() - 1
                } else {
                    selected_index - 1
                };
                needs_render = true;
            }
            KeyCode::Down => {
                selected_index = (selected_index + 1) % providers.len();
                needs_render = true;
            }
            KeyCode::Char(' ') => {
                selected[selected_index] = !selected[selected_index];
                needs_render = true;
            }
            KeyCode::Enter => {
                clear_rendered_block(rendered_lines)?;
                drop(raw_mode);
                let target_indices = selected_or_focused_indices(&selected, selected_index);
                handle_auth_provider_action(runtime, &providers, &target_indices)?;
                refresh_active_footer(runtime);
                providers = auth_provider_settings_items(&runtime.model_settings_items());
                if providers.is_empty() {
                    return Ok(());
                }
                selected = vec![false; providers.len()];
                selected_index = selected_index.min(providers.len() - 1);
                rendered_lines = 0;
                raw_mode = RawModeGuard::enable()?;
                needs_render = true;
            }
            KeyCode::Esc => {
                clear_rendered_block(rendered_lines)?;
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

fn render_auth_settings_selector_manual(
    providers: &[AuthProviderSettingsItem],
    selected: &[bool],
    selected_index: usize,
    previous_lines: usize,
) -> io::Result<usize> {
    if previous_lines > 0 {
        clear_rendered_block(previous_lines)?;
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "{} {}",
        bold("settings auth"),
        dim("↑/↓ move  Space select  Enter actions  Esc cancel  Ctrl+C exit")
    ));
    lines.push(String::new());

    for (index, provider) in providers.iter().enumerate() {
        let pointer = if index == selected_index { ">" } else { " " };
        let checkbox = if selected[index] { "[*]" } else { "[ ]" };
        let line = format!("{pointer} {checkbox} {}", provider.provider);
        let line = if provider.is_enabled {
            line
        } else {
            dim(&line)
        };
        lines.push(if index == selected_index {
            accent(&line)
        } else {
            line
        });
    }

    let rendered_lines = print_fitted_terminal_lines(&lines)?;
    render_active_footer()?;
    io::stdout().flush()?;
    Ok(rendered_lines)
}

fn render_auth_settings_selector_view(
    frame: &mut Frame<'_>,
    providers: &[AuthProviderSettingsItem],
    selected: &[bool],
    selected_index: usize,
) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    let hint_height = (area.height >= 3).then_some(1).unwrap_or(0);
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(hint_height),
        ])
        .split(area);

    Paragraph::new(Line::from(Span::styled(
        "settings auth",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )))
    .render(areas[0], frame.buffer_mut());

    let body_height = usize::from(areas[1].height);
    let selected_index = selected_index.min(providers.len().saturating_sub(1));
    let start = selected_index.saturating_sub(body_height.saturating_sub(1));
    let lines = providers
        .iter()
        .enumerate()
        .skip(start)
        .take(body_height)
        .map(|(index, provider)| {
            let is_selected = index == selected_index;
            let is_checked = selected.get(index).copied().unwrap_or(false);
            let style = if is_selected {
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD)
            } else if provider.is_enabled {
                Style::default()
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::from(vec![
                Span::styled(if is_selected { "> " } else { "  " }, style),
                Span::styled(if is_checked { "[*] " } else { "[ ] " }, style),
                Span::styled(provider.provider.clone(), style),
            ])
        })
        .collect::<Vec<_>>();
    Paragraph::new(Text::from(lines)).render(areas[1], frame.buffer_mut());

    if hint_height > 0 {
        Paragraph::new(Line::from(Span::styled(
            "↑/↓ move   space select   enter actions   escape cancel   ctrl+c exit",
            Style::default().fg(Color::DarkGray),
        )))
        .render(areas[2], frame.buffer_mut());
    }
}

fn open_auth_settings_line(
    runtime: &mut AppRuntime,
    providers: &[AuthProviderSettingsItem],
) -> io::Result<()> {
    println!("Configured providers:");
    for (index, provider) in providers.iter().enumerate() {
        let line = format!("{}) [ ] {}", index + 1, provider.provider);
        println!(
            "{}",
            if provider.is_enabled {
                line
            } else {
                dim(&line)
            }
        );
    }

    let Some(line) = read_cancelable_line("select providers, comma-separated: ")? else {
        println!("settings auth cancelled");
        return Ok(());
    };
    let target_indices = parse_model_number_list(line.trim(), providers.len())?;
    if target_indices.is_empty() {
        return Ok(());
    }

    handle_auth_provider_action(runtime, providers, &target_indices)
}

fn handle_auth_provider_action(
    runtime: &mut AppRuntime,
    providers: &[AuthProviderSettingsItem],
    target_indices: &[usize],
) -> io::Result<()> {
    let labels = ["enable", "disable", "remove"];
    let selected = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        select_with_keys("Select action:", &labels)?
    } else {
        println!("Actions for {} provider(s):", target_indices.len());
        println!("1) enable");
        println!("2) disable");
        println!("3) remove");
        let Some(line) = read_cancelable_line("select action: ")? else {
            return Ok(());
        };
        match line.trim() {
            "1" | "enable" => Some(0),
            "2" | "disable" => Some(1),
            "3" | "remove" => Some(2),
            "" => None,
            _ => {
                println!("invalid action");
                None
            }
        }
    };

    match selected {
        Some(0) => set_providers_enabled(runtime, providers, target_indices, true),
        Some(1) => set_providers_enabled(runtime, providers, target_indices, false),
        Some(2) => {
            for index in target_indices.iter().rev() {
                remove_provider_auth_or_models(runtime, &providers[*index]);
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn set_providers_enabled(
    runtime: &mut AppRuntime,
    providers: &[AuthProviderSettingsItem],
    target_indices: &[usize],
    enabled: bool,
) -> io::Result<()> {
    let mut enabled_indices = runtime
        .model_settings_items()
        .into_iter()
        .filter(|model| model.is_enabled)
        .map(|model| model.index)
        .collect::<Vec<_>>();

    if enabled {
        for index in target_indices {
            for model_index in &providers[*index].model_indices {
                if !enabled_indices.contains(model_index) {
                    enabled_indices.push(*model_index);
                }
            }
        }
    } else {
        for index in target_indices {
            for model_index in &providers[*index].model_indices {
                enabled_indices.retain(|index| index != model_index);
            }
        }
    }

    match runtime.set_enabled_model_indices(&enabled_indices) {
        Ok(()) => print_fitted_terminal_line(format!(
            "{} provider(s) {}",
            target_indices.len(),
            if enabled { "enabled" } else { "disabled" }
        ))?,
        Err(error) => print_fitted_terminal_line(format!("error: {error}"))?,
    }
    Ok(())
}

fn remove_provider_auth_or_models(runtime: &mut AppRuntime, provider: &AuthProviderSettingsItem) {
    if provider.has_built_in {
        if let Some(index) = provider.model_indices.first() {
            match runtime.delete_model(*index) {
                Ok(message) => {
                    let _ = print_fitted_terminal_line(message);
                }
                Err(error) => {
                    let _ = print_fitted_terminal_line(format!("error: {error}"));
                }
            }
        }
        return;
    }

    for index in provider.model_indices.iter().rev() {
        match runtime.delete_model(*index) {
            Ok(message) => {
                let _ = print_fitted_terminal_line(message);
            }
            Err(error) => {
                let _ = print_fitted_terminal_line(format!("error: {error}"));
            }
        }
    }
}

fn parse_model_number_list(input: &str, model_count: usize) -> io::Result<Vec<usize>> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let mut indices = Vec::new();
    for value in input
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let Ok(index) = value.parse::<usize>() else {
            println!("invalid model number: {value}");
            return Ok(Vec::new());
        };
        if index == 0 || index > model_count {
            println!("model number out of range: {value}");
            return Ok(Vec::new());
        }
        let index = index - 1;
        if !indices.contains(&index) {
            indices.push(index);
        }
    }
    Ok(indices)
}

fn edit_model_settings_with_keys(
    runtime: &mut AppRuntime,
    mut models: Vec<ModelSettingsItem>,
) -> io::Result<()> {
    refresh_active_footer(runtime);
    let mut selected_index = 0usize;
    let mut selected = vec![false; models.len()];
    let mut raw_mode = RawModeGuard::enable()?;
    let mut terminal = match inline_terminal(inline_height_for_lines(
        models.len().saturating_mul(2).saturating_add(3),
    )) {
        Ok(terminal) => terminal,
        Err(_) => {
            reset_terminal_viewport()?;
            return edit_model_settings_manual_loop(runtime, models, selected_index, selected);
        }
    };

    loop {
        terminal.draw(|frame| {
            render_model_settings_selector_view(frame, &models, &selected, selected_index)
        })?;

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
            KeyCode::Char(' ') => {
                selected[selected_index] = !selected[selected_index];
            }
            KeyCode::Enter => {
                finish_inline_terminal(terminal)?;
                drop(raw_mode);
                let target_indices = selected_or_focused_indices(&selected, selected_index);
                handle_model_settings_action(runtime, &models, &target_indices)?;
                refresh_active_footer(runtime);
                models = enabled_provider_model_settings_items(&runtime.model_settings_items());
                if models.is_empty() {
                    return Ok(());
                }
                selected = vec![false; models.len()];
                selected_index = selected_index.min(models.len() - 1);
                raw_mode = RawModeGuard::enable()?;
                terminal = match inline_terminal(inline_height_for_lines(
                    models.len().saturating_mul(2).saturating_add(3),
                )) {
                    Ok(terminal) => terminal,
                    Err(_) => {
                        reset_terminal_viewport()?;
                        return edit_model_settings_manual_loop(
                            runtime,
                            models,
                            selected_index,
                            selected,
                        );
                    }
                };
            }
            KeyCode::Esc => {
                finish_inline_terminal(terminal)?;
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

fn edit_model_settings_manual_loop(
    runtime: &mut AppRuntime,
    mut models: Vec<ModelSettingsItem>,
    mut selected_index: usize,
    mut selected: Vec<bool>,
) -> io::Result<()> {
    let mut raw_mode = RawModeGuard::enable()?;
    let mut rendered_lines = 0usize;
    let mut needs_render = true;

    loop {
        if needs_render {
            rendered_lines = render_model_settings_selector_manual(
                &models,
                &selected,
                selected_index,
                rendered_lines,
            )?;
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
            KeyCode::Char(' ') => {
                selected[selected_index] = !selected[selected_index];
                needs_render = true;
            }
            KeyCode::Enter => {
                clear_rendered_block(rendered_lines)?;
                drop(raw_mode);
                let target_indices = selected_or_focused_indices(&selected, selected_index);
                handle_model_settings_action(runtime, &models, &target_indices)?;
                refresh_active_footer(runtime);
                models = enabled_provider_model_settings_items(&runtime.model_settings_items());
                if models.is_empty() {
                    return Ok(());
                }
                selected = vec![false; models.len()];
                selected_index = selected_index.min(models.len() - 1);
                rendered_lines = 0;
                raw_mode = RawModeGuard::enable()?;
                needs_render = true;
            }
            KeyCode::Esc => {
                clear_rendered_block(rendered_lines)?;
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

fn render_model_settings_selector_manual(
    models: &[ModelSettingsItem],
    selected: &[bool],
    selected_index: usize,
    previous_lines: usize,
) -> io::Result<usize> {
    if previous_lines > 0 {
        clear_rendered_block(previous_lines)?;
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "{} {}",
        bold("settings model"),
        dim("↑/↓ move  Space select  Enter actions  Esc cancel  Ctrl+C exit")
    ));
    lines.push(String::new());

    let mut last_provider = "";
    for (index, model) in models.iter().enumerate() {
        if model.provider != last_provider {
            lines.push(accent(&model.provider));
            last_provider = &model.provider;
        }
        let pointer = if index == selected_index { ">" } else { " " };
        let checkbox = if selected[index] { "[*]" } else { "[ ]" };
        let line = format!("{pointer} {checkbox} {}", model.id);
        let line = if model.is_enabled { line } else { dim(&line) };
        lines.push(if index == selected_index {
            accent(&line)
        } else {
            line
        });
    }

    let rendered_lines = print_fitted_terminal_lines(&lines)?;
    render_active_footer()?;
    io::stdout().flush()?;
    Ok(rendered_lines)
}

fn render_model_settings_selector_view(
    frame: &mut Frame<'_>,
    models: &[ModelSettingsItem],
    selected: &[bool],
    selected_index: usize,
) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    let hint_height = (area.height >= 3).then_some(1).unwrap_or(0);
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(hint_height),
        ])
        .split(area);

    Paragraph::new(Line::from(Span::styled(
        "settings model",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )))
    .render(areas[0], frame.buffer_mut());

    let (rows, selected_row) = model_settings_rows(models, selected, selected_index);
    let body_height = usize::from(areas[1].height);
    let start = selected_row.saturating_sub(body_height.saturating_sub(1));
    let lines = rows
        .into_iter()
        .skip(start)
        .take(body_height)
        .collect::<Vec<_>>();
    Paragraph::new(Text::from(lines)).render(areas[1], frame.buffer_mut());

    if hint_height > 0 {
        Paragraph::new(Line::from(Span::styled(
            "↑/↓ move   space select   enter actions   escape cancel   ctrl+c exit",
            Style::default().fg(Color::DarkGray),
        )))
        .render(areas[2], frame.buffer_mut());
    }
}

fn model_settings_rows(
    models: &[ModelSettingsItem],
    selected: &[bool],
    selected_index: usize,
) -> (Vec<Line<'static>>, usize) {
    let mut rows = Vec::new();
    let mut selected_row = 0usize;
    let mut last_provider = "";

    for (index, model) in models.iter().enumerate() {
        if model.provider != last_provider {
            rows.push(Line::from(Span::styled(
                model.provider.clone(),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )));
            last_provider = &model.provider;
        }

        let is_selected = index == selected_index;
        let is_checked = selected.get(index).copied().unwrap_or(false);
        let style = if is_selected {
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD)
        } else if model.is_enabled {
            Style::default()
        } else {
            Style::default().fg(Color::DarkGray)
        };
        if is_selected {
            selected_row = rows.len();
        }
        rows.push(Line::from(vec![
            Span::styled(if is_selected { "> " } else { "  " }, style),
            Span::styled(if is_checked { "[*] " } else { "[ ] " }, style),
            Span::styled(model.id.clone(), style),
        ]));
    }

    (rows, selected_row)
}

fn edit_model_settings_line(
    runtime: &mut AppRuntime,
    models: &[ModelSettingsItem],
) -> io::Result<()> {
    println!("Model visibility:");
    let mut last_provider = "";
    for (index, model) in models.iter().enumerate() {
        if model.provider != last_provider {
            println!("{}", model.provider);
            last_provider = &model.provider;
        }
        let line = format!("{}) [ ] {}", index + 1, model.id);
        println!("{}", if model.is_enabled { line } else { dim(&line) });
    }

    let Some(line) = read_cancelable_line("select models, comma-separated: ")? else {
        println!("model settings cancelled");
        return Ok(());
    };
    let target_indices = parse_model_number_list(line.trim(), models.len())?;
    if target_indices.is_empty() {
        return Ok(());
    }

    handle_model_settings_action(runtime, models, &target_indices)
}

fn handle_model_settings_action(
    runtime: &mut AppRuntime,
    models: &[ModelSettingsItem],
    target_indices: &[usize],
) -> io::Result<()> {
    let labels = ["enable", "disable"];
    let selected = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        select_with_keys("Select action:", &labels)?
    } else {
        println!("Actions for {} model(s):", target_indices.len());
        println!("1) enable");
        println!("2) disable");
        let Some(line) = read_cancelable_line("select action: ")? else {
            return Ok(());
        };
        match line.trim() {
            "1" | "enable" => Some(0),
            "2" | "disable" => Some(1),
            "" => None,
            _ => {
                println!("invalid action");
                None
            }
        }
    };

    match selected {
        Some(0) => set_model_items_enabled(runtime, models, target_indices, true),
        Some(1) => set_model_items_enabled(runtime, models, target_indices, false),
        _ => Ok(()),
    }
}

fn set_model_items_enabled(
    runtime: &mut AppRuntime,
    models: &[ModelSettingsItem],
    target_indices: &[usize],
    enabled: bool,
) -> io::Result<()> {
    let mut enabled_indices = runtime
        .model_settings_items()
        .into_iter()
        .filter(|model| model.is_enabled)
        .map(|model| model.index)
        .collect::<Vec<_>>();

    if enabled {
        for index in target_indices {
            let model_index = models[*index].index;
            if !enabled_indices.contains(&model_index) {
                enabled_indices.push(model_index);
            }
        }
    } else {
        for index in target_indices {
            let model_index = models[*index].index;
            enabled_indices.retain(|index| *index != model_index);
        }
    }

    match runtime.set_enabled_model_indices(&enabled_indices) {
        Ok(()) => print_fitted_terminal_line(format!(
            "{} model(s) {}",
            target_indices.len(),
            if enabled { "enabled" } else { "disabled" }
        ))?,
        Err(error) => print_fitted_terminal_line(format!("error: {error}"))?,
    }
    Ok(())
}

fn clear_rendered_block(lines: usize) -> io::Result<()> {
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

struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

fn exit_process() -> ! {
    reset_scroll_region();
    let _ = terminal::disable_raw_mode();
    println!("bye");
    std::process::exit(0);
}

fn read_cancelable_line(prompt: &str) -> io::Result<Option<String>> {
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

    print!("{prompt}");
    render_footer(footer)?;
    io::stdout().flush()?;

    loop {
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
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
                    redraw_input_with_completion(prompt, &input, &mut completion_lines, footer)?;
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
                    redraw_input_with_completion(prompt, &input, &mut completion_lines, footer)?;
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
                    redraw_input_with_completion(prompt, &input, &mut completion_lines, footer)?;
                }
                KeyCode::Char(value) => {
                    history_index = None;
                    input.push(value);
                    redraw_input_with_completion(prompt, &input, &mut completion_lines, footer)?;
                }
                _ => {}
            },
            Event::Paste(value) => {
                history_index = None;
                input.push_str(&value);
                redraw_input_with_completion(prompt, &input, &mut completion_lines, footer)?;
            }
            Event::Resize(_, _) => {
                reset_terminal_viewport()?;
            }
            _ => {}
        }
    }
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

fn redraw_current_prompt_line(prompt: &str, input: &str) -> io::Result<()> {
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

fn reset_terminal_viewport() -> io::Result<()> {
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

fn open_model_menu_line(runtime: &mut AppRuntime) -> io::Result<()> {
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

fn open_auth_menu(runtime: &mut AppRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    let method = select_auth_method()?;
    match method {
        Some(AuthMethod::Subscription) => open_subscription_auth_menu(runtime),
        Some(AuthMethod::ApiKey) => open_api_key_auth_menu(runtime),
        None => Ok(()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AuthMethod {
    Subscription,
    ApiKey,
}

fn select_auth_method() -> io::Result<Option<AuthMethod>> {
    let labels = ["Use a subscription", "Use an API key"];

    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return Ok(
            select_with_keys("Select authentication method:", &labels)?.map(|index| {
                if index == 0 {
                    AuthMethod::Subscription
                } else {
                    AuthMethod::ApiKey
                }
            }),
        );
    }

    println!("Select authentication method:");
    println!("1) Use a subscription");
    println!("2) Use an API key");
    let Some(line) = read_cancelable_line("select auth method: ")? else {
        return Ok(None);
    };
    match line.trim() {
        "" => Ok(None),
        "1" => Ok(Some(AuthMethod::Subscription)),
        "2" => Ok(Some(AuthMethod::ApiKey)),
        _ => {
            println!("invalid authentication method");
            Ok(None)
        }
    }
}

fn open_subscription_auth_menu(runtime: &mut AppRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    let providers = runtime.subscription_providers();
    let labels = providers
        .iter()
        .map(|provider| {
            let status = auth_status_label(provider.has_subscription);
            format!("{}  {status}", provider.name)
        })
        .collect::<Vec<_>>();
    let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();

    let selected = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        select_with_keys("Select subscription provider:", &label_refs)?
    } else {
        println!("Select subscription provider:");
        for (index, label) in labels.iter().enumerate() {
            println!("{}) {label}", index + 1);
        }
        let Some(line) = read_cancelable_line("select provider: ")? else {
            return Ok(());
        };
        match line.trim().parse::<usize>() {
            Ok(value) if value > 0 && value <= labels.len() => Some(value - 1),
            _ => None,
        }
    };

    let Some(index) = selected else {
        println!("subscription login cancelled");
        return Ok(());
    };

    let provider = &providers[index];
    match provider.provider.as_str() {
        "anthropic" => login_anthropic_subscription(runtime, &provider.provider),
        "github-copilot" => login_github_copilot_subscription(runtime, &provider.provider),
        "openai-codex" => {
            println!(
                "subscription login for {} needs callback-server OAuth and is not implemented yet.",
                provider.name
            );
            println!("provider id: {}", provider.provider);
            Ok(())
        }
        _ => {
            println!("unknown subscription provider: {}", provider.provider);
            Ok(())
        }
    }
}

fn login_anthropic_subscription(runtime: &mut AppRuntime, provider_id: &str) -> io::Result<()> {
    refresh_active_footer(runtime);
    println!("Anthropic subscription login");
    let flow = match start_anthropic_oauth_flow() {
        Ok(flow) => flow,
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(());
        }
    };

    println!("Open this URL in your browser:");
    println!("{}", flow.url);
    println!("Complete login in your browser.");
    println!("Waiting for browser callback...");

    let authorization = match wait_for_anthropic_authorization(&flow)? {
        Some(authorization) => authorization,
        None => {
            println!("subscription login cancelled");
            return Ok(());
        }
    };

    println!("Exchanging authorization code for tokens...");
    let credential = match finish_anthropic_oauth_flow(&flow, authorization) {
        Ok(credential) => credential,
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(());
        }
    };

    match runtime.set_oauth_credential(provider_id, credential) {
        Ok(()) => println!(
            "Logged in to Anthropic. Credentials saved to {}",
            runtime.auth_path()
        ),
        Err(error) => eprintln!("error: {error}"),
    }
    Ok(())
}

fn wait_for_anthropic_authorization(
    flow: &AnthropicOAuthFlow,
) -> io::Result<Option<AuthorizationCode>> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return wait_for_anthropic_authorization_tty(flow);
    }

    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    if let Some(authorization) = parse_authorization_input_or_print(&line) {
        return Ok(Some(authorization));
    }

    match flow.wait_for_callback(Duration::from_secs(10 * 60)) {
        Ok(authorization) => Ok(Some(authorization)),
        Err(error) => {
            eprintln!("error: {error}");
            Ok(None)
        }
    }
}

fn wait_for_anthropic_authorization_tty(
    flow: &AnthropicOAuthFlow,
) -> io::Result<Option<AuthorizationCode>> {
    println!("Paste final redirect URL any time if callback does not return.");
    let prompt = "redirect URL: ";
    print!("{prompt}");
    io::stdout().flush()?;

    let _raw_mode = RawModeGuard::enable()?;
    let deadline = Instant::now() + Duration::from_secs(10 * 60);
    let mut input = String::new();

    loop {
        match flow.poll_callback(Duration::from_millis(50)) {
            Ok(Some(authorization)) => {
                print_terminal_newline()?;
                print_fitted_terminal_line("Browser callback received.")?;
                return Ok(Some(authorization));
            }
            Ok(None) => {}
            Err(error) => {
                print_terminal_newline()?;
                print_fitted_terminal_line(format!("error: {error}"))?;
                return Ok(None);
            }
        }

        if Instant::now() >= deadline {
            print_terminal_newline()?;
            print_fitted_terminal_line("error: timed out waiting for OAuth callback")?;
            return Ok(None);
        }

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Enter => {
                    print_terminal_newline()?;
                    if input.trim().is_empty() {
                        redraw_current_prompt_line(prompt, &input)?;
                        continue;
                    }
                    return Ok(parse_authorization_input_or_print(&input));
                }
                KeyCode::Esc => {
                    print_terminal_newline()?;
                    return Ok(None);
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    print_terminal_newline()?;
                    exit_process();
                }
                KeyCode::Backspace if input.pop().is_some() => {
                    redraw_current_prompt_line(prompt, &input)?;
                }
                KeyCode::Char(value) => {
                    input.push(value);
                    redraw_current_prompt_line(prompt, &input)?;
                }
                _ => {}
            },
            Event::Paste(value) => {
                input.push_str(&value);
                redraw_current_prompt_line(prompt, &input)?;
            }
            Event::Resize(_, _) => {
                reset_terminal_viewport()?;
            }
            _ => {}
        }
    }
}

fn parse_authorization_input_or_print(input: &str) -> Option<AuthorizationCode> {
    match parse_authorization_input(input) {
        Ok(authorization) => authorization,
        Err(error) => {
            eprintln!("error: {error}");
            None
        }
    }
}

fn login_github_copilot_subscription(
    runtime: &mut AppRuntime,
    provider_id: &str,
) -> io::Result<()> {
    refresh_active_footer(runtime);
    println!("GitHub Copilot subscription login");
    println!("Leave enterprise domain blank for github.com.");
    let Some(enterprise_input) = read_cancelable_line("GitHub Enterprise URL/domain: ")? else {
        println!("subscription login cancelled");
        return Ok(());
    };
    let enterprise_domain = normalize_github_domain(&enterprise_input);
    if !enterprise_input.trim().is_empty() && enterprise_domain.is_none() {
        println!("invalid GitHub Enterprise URL/domain");
        return Ok(());
    }

    let flow = match start_github_copilot_device_flow(enterprise_domain.as_deref()) {
        Ok(flow) => flow,
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(());
        }
    };

    println!("Open this URL in your browser:");
    println!("{}", flow.verification_uri);
    println!("Enter code: {}", flow.user_code);

    let credential = match finish_github_copilot_device_flow(&flow, |message| println!("{message}"))
    {
        Ok(credential) => credential,
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(());
        }
    };

    match runtime.set_oauth_credential(provider_id, credential) {
        Ok(()) => println!(
            "Logged in to GitHub Copilot. Credentials saved to {}",
            runtime.auth_path()
        ),
        Err(error) => eprintln!("error: {error}"),
    }
    Ok(())
}

fn select_with_keys(title: &str, labels: &[&str]) -> io::Result<Option<usize>> {
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
        "↑/↓ navigate   space select   enter confirm   escape cancel   ctrl+c exit",
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

    let hint_height = (area.height >= 3).then_some(1).unwrap_or(0);
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
            "↑/↓ navigate   enter confirm   escape cancel   ctrl+c exit",
            Style::default().fg(Color::DarkGray),
        )))
        .render(areas[2], frame.buffer_mut());
    }
}

fn open_api_key_auth_menu(runtime: &mut AppRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    let providers = runtime.auth_providers();
    println!("auth file: {}", runtime.auth_path());
    println!("models file: {}", runtime.models_path());

    let selected_index = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        let mut labels = providers
            .iter()
            .map(|provider| {
                let status = auth_status_label(provider.has_token);
                format!("{}  {status}", provider.provider)
            })
            .collect::<Vec<_>>();
        labels.push("add OpenAI-compatible model".to_string());
        let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
        select_with_keys("Select provider to configure:", &label_refs)?
    } else {
        open_api_key_auth_menu_line(&providers)?
    };

    let Some(selected_index) = selected_index else {
        println!("auth configuration cancelled");
        return Ok(());
    };

    if selected_index == providers.len() {
        return add_openai_compatible_model(runtime);
    }

    let Some(selected) = providers.get(selected_index) else {
        println!("invalid provider selection");
        return Ok(());
    };
    configure_api_key_provider(runtime, &selected.provider)?;
    refresh_active_footer(runtime);

    Ok(())
}

fn open_api_key_auth_menu_line(
    providers: &[crate::app::AuthProviderInfo],
) -> io::Result<Option<usize>> {
    for (index, provider) in providers.iter().enumerate() {
        let status = auth_status_label(provider.has_token);
        println!("{}) {}  {}", index + 1, provider.provider, status);
    }
    println!("{}) add OpenAI-compatible model", providers.len() + 1);

    let Some(line) = read_cancelable_line("select auth action: ")? else {
        return Ok(None);
    };
    let choice = line.trim();
    if choice.is_empty() {
        return Ok(None);
    }

    match choice.parse::<usize>() {
        Ok(value) if value > 0 && value <= providers.len() + 1 => Ok(Some(value - 1)),
        _ => {
            println!("invalid provider selection");
            Ok(None)
        }
    }
}

fn configure_api_key_provider(runtime: &mut AppRuntime, provider: &str) -> io::Result<()> {
    refresh_active_footer(runtime);
    println!("enter token for {provider}.");
    println!("leave blank to remove the stored token.");
    let Some(line) = read_cancelable_line("token: ")? else {
        println!("auth configuration cancelled");
        return Ok(());
    };
    let token = line.trim();

    if token.is_empty() {
        match runtime.remove_auth_token(provider) {
            Ok(()) => println!("removed auth token for {provider}"),
            Err(error) => eprintln!("error: {error}"),
        }
    } else {
        match runtime.set_auth_token(provider, token) {
            Ok(()) => println!("saved auth token for {provider}"),
            Err(error) => eprintln!("error: {error}"),
        }
    }

    Ok(())
}

fn add_openai_compatible_model(runtime: &mut AppRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    let Some(provider) = prompt_required("provider id, e.g. deepseek")? else {
        return Ok(());
    };
    let Some(model_id) = prompt_required("model id, e.g. deepseek-chat")? else {
        return Ok(());
    };
    let Some(base_url) = prompt_required("base url, e.g. https://api.deepseek.com/v1")? else {
        return Ok(());
    };

    println!("enter API key for {provider}.");
    println!("leave blank to add the model without storing a key.");
    let Some(api_key) = read_cancelable_line("api key: ")? else {
        println!("model configuration cancelled");
        return Ok(());
    };

    match runtime.add_openai_compatible_model(&provider, &model_id, &base_url, api_key.trim()) {
        Ok(info) => {
            println!(
                "added model {} / {} at index {}",
                info.provider,
                info.model_id,
                info.index + 1
            );
            println!("selected model: {}", runtime.model_label());
        }
        Err(error) => eprintln!("error: {error}"),
    }

    Ok(())
}

fn prompt_required(label: &str) -> io::Result<Option<String>> {
    let Some(line) = read_cancelable_line(&format!("{label}: "))? else {
        println!("model configuration cancelled");
        return Ok(None);
    };
    let value = line.trim().to_string();
    if value.is_empty() {
        println!("required value was empty");
        return Ok(None);
    }
    Ok(Some(value))
}

fn open_session_menu(runtime: &mut AppRuntime) -> io::Result<()> {
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

fn print_recent_messages(runtime: &AppRuntime) {
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

struct EventRenderer {
    show_prompt_details: bool,
    message_started: bool,
    reasoning_started: bool,
    reasoning_ended_with_newline: bool,
    leading_blank_available: bool,
}

impl EventRenderer {
    fn new(show_prompt_details: bool, leading_blank_printed: bool) -> Self {
        Self {
            show_prompt_details,
            message_started: false,
            reasoning_started: false,
            reasoning_ended_with_newline: false,
            leading_blank_available: leading_blank_printed,
        }
    }

    fn render(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::AgentStart => {}
            AgentEvent::MessageStart { .. } => {}
            AgentEvent::MessageDelta { delta } => {
                let finished_reasoning = self.finish_reasoning();
                if !self.message_started && !finished_reasoning && !self.leading_blank_available {
                    println!();
                }
                self.leading_blank_available = false;
                self.message_started = true;
                print!("{delta}");
                let _ = io::stdout().flush();
            }
            AgentEvent::ReasoningDelta { delta } => {
                if self.show_prompt_details {
                    if !self.reasoning_started && !self.leading_blank_available {
                        println!();
                    }
                    self.leading_blank_available = false;
                    self.reasoning_started = true;
                    self.reasoning_ended_with_newline = delta.ends_with('\n');
                    print!("{}", thinking(&delta));
                    let _ = io::stdout().flush();
                }
            }
            AgentEvent::MessageEnd { .. } => {
                self.finish_reasoning();
                if self.message_started {
                    println!("\n");
                    self.message_started = false;
                }
            }
            AgentEvent::ToolCallStart {
                name, arguments, ..
            } => {
                self.finish_reasoning();
                self.leading_blank_available = false;
                println!("{} {name}: {arguments:?}", dim("tool"));
            }
            AgentEvent::ToolCallEnd {
                name,
                content,
                is_error,
                ..
            } => {
                self.finish_reasoning();
                self.leading_blank_available = false;
                let status = if is_error { "error" } else { "ok" };
                println!("{} {name} {status}:\n{content}", dim("tool"));
            }
            AgentEvent::AgentEnd => {}
            AgentEvent::Usage { .. } => {}
            AgentEvent::Error { message } => {
                self.finish_reasoning();
                self.leading_blank_available = false;
                eprintln!("error: {message}");
            }
        }
    }

    fn finish_reasoning(&mut self) -> bool {
        if self.reasoning_started {
            if !self.reasoning_ended_with_newline {
                println!();
            }
            self.reasoning_started = false;
            self.reasoning_ended_with_newline = false;
            return true;
        }
        false
    }
}

fn event_has_visible_output(event: &AgentEvent, show_prompt_details: bool) -> bool {
    match event {
        AgentEvent::MessageDelta { .. }
        | AgentEvent::MessageEnd { .. }
        | AgentEvent::ToolCallStart { .. }
        | AgentEvent::ToolCallEnd { .. }
        | AgentEvent::Error { .. } => true,
        AgentEvent::ReasoningDelta { .. } => show_prompt_details,
        AgentEvent::AgentStart
        | AgentEvent::MessageStart { .. }
        | AgentEvent::Usage { .. }
        | AgentEvent::AgentEnd => false,
    }
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

fn current_directory_label() -> String {
    env::current_dir()
        .map(|path| path.display().to_string().replace('\\', "/"))
        .unwrap_or_else(|_| ".".to_string())
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

fn strip_ansi(text: &str) -> String {
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

fn bold(text: &str) -> String {
    format!("\x1b[1m{text}\x1b[0m")
}

fn dim(text: &str) -> String {
    format!("\x1b[2m{text}\x1b[0m")
}

fn thinking(text: &str) -> String {
    format!("\x1b[2;3m{text}\x1b[0m")
}

fn accent(text: &str) -> String {
    format!("\x1b[35;1m{text}\x1b[0m")
}

fn cyan(text: &str) -> String {
    format!("\x1b[36;1m{text}\x1b[0m")
}

fn success(text: &str) -> String {
    format!("\x1b[32;1m{text}\x1b[0m")
}

fn auth_status_label(is_configured: bool) -> String {
    if is_configured {
        success("configured")
    } else {
        "missing".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let (_rows, selected_row) = model_settings_rows(&models, &[false, true], 1);

        assert_eq!(selected_row, 3);
    }
}
