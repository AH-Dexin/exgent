use std::io::{self, IsTerminal, Write};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Widget},
    Frame,
};

use exgent_core::{
    AppRuntimeHost, ModelSettingsItem, ThemeRgb, ThemeSettings, LANGUAGE_OPTIONS, THEME_PRESETS,
};

use super::settings_actions::{
    auth_provider_settings_items, enabled_indices_after_model_action,
    enabled_indices_after_provider_action, enabled_provider_model_settings_items,
    provider_model_indices_for_removal, AuthProviderSettingsItem,
};
use super::{
    accent, bold, clear_rendered_block, dim, exit_process, finish_inline_terminal,
    inline_height_for_lines, inline_terminal, print_fitted_terminal_line,
    print_fitted_terminal_lines, read_cancelable_line, refresh_active_footer, render_active_footer,
    reset_terminal_viewport, select_with_keys, selected_or_focused_indices, RawModeGuard,
};

type TuiRuntime = AppRuntimeHost;

pub(super) fn open_model_settings(runtime: &mut TuiRuntime) -> io::Result<()> {
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

pub(super) fn open_settings_menu(runtime: &mut TuiRuntime) -> io::Result<()> {
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

pub(super) fn open_theme_settings(runtime: &mut TuiRuntime) -> io::Result<()> {
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

pub(super) fn open_language_settings(runtime: &mut TuiRuntime) -> io::Result<()> {
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

pub(super) fn open_auth_settings(runtime: &mut TuiRuntime) -> io::Result<()> {
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

fn open_auth_settings_with_keys(
    runtime: &mut TuiRuntime,
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
    runtime: &mut TuiRuntime,
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
        dim("up/down move  Space select  Enter actions  Esc cancel  Ctrl+C exit")
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
            "up/down move   space select   enter actions   escape cancel   ctrl+c exit",
            Style::default().fg(Color::DarkGray),
        )))
        .render(areas[2], frame.buffer_mut());
    }
}

fn open_auth_settings_line(
    runtime: &mut TuiRuntime,
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
    runtime: &mut TuiRuntime,
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
    runtime: &mut TuiRuntime,
    providers: &[AuthProviderSettingsItem],
    target_indices: &[usize],
    enabled: bool,
) -> io::Result<()> {
    let current_models = runtime.model_settings_items();
    let enabled_indices =
        enabled_indices_after_provider_action(&current_models, providers, target_indices, enabled);

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

fn remove_provider_auth_or_models(runtime: &mut TuiRuntime, provider: &AuthProviderSettingsItem) {
    for index in provider_model_indices_for_removal(provider) {
        match runtime.delete_model(index) {
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
    runtime: &mut TuiRuntime,
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
    runtime: &mut TuiRuntime,
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
        dim("up/down move  Space select  Enter actions  Esc cancel  Ctrl+C exit")
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
            "up/down move   space select   enter actions   escape cancel   ctrl+c exit",
            Style::default().fg(Color::DarkGray),
        )))
        .render(areas[2], frame.buffer_mut());
    }
}

pub(super) fn model_settings_rows(
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
    runtime: &mut TuiRuntime,
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
    runtime: &mut TuiRuntime,
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
    runtime: &mut TuiRuntime,
    models: &[ModelSettingsItem],
    target_indices: &[usize],
    enabled: bool,
) -> io::Result<()> {
    let current_models = runtime.model_settings_items();
    let enabled_indices =
        enabled_indices_after_model_action(&current_models, models, target_indices, enabled);

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
