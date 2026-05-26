use exgent_core::{AppRuntimeHost, ThemeRgb, ThemeSettings, THEME_PRESETS};

use super::state::{AddModelFormState, CustomThemeState, TuiApp};

type TuiRuntime = AppRuntimeHost;

pub(super) fn active_add_model_field_mut(state: &mut AddModelFormState) -> &mut String {
    match state.field {
        0 => &mut state.provider,
        1 => &mut state.model_id,
        2 => &mut state.base_url,
        _ => &mut state.api_key,
    }
}

pub(super) fn active_theme_field_mut(state: &mut CustomThemeState) -> &mut String {
    match state.field {
        0 => &mut state.red,
        1 => &mut state.green,
        _ => &mut state.blue,
    }
}

pub(super) fn paste_theme_value(state: &mut CustomThemeState, value: &str) {
    let components = value
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .take(3)
        .collect::<Vec<_>>();

    if components.len() >= 3 {
        state.red = components[0].chars().take(3).collect();
        state.green = components[1].chars().take(3).collect();
        state.blue = components[2].chars().take(3).collect();
        return;
    }

    let field = active_theme_field_mut(state);
    for ch in value.chars().filter(|ch| ch.is_ascii_digit()) {
        if field.len() >= 3 {
            break;
        }
        field.push(ch);
    }
}

pub(super) fn parse_custom_theme(state: &CustomThemeState) -> Result<ThemeSettings, String> {
    let red = parse_rgb_component("red", &state.red)?;
    let green = parse_rgb_component("green", &state.green)?;
    let blue = parse_rgb_component("blue", &state.blue)?;
    Ok(ThemeSettings::custom(ThemeRgb::new(red, green, blue)))
}

fn parse_rgb_component(label: &str, value: &str) -> Result<u8, String> {
    value
        .trim()
        .parse::<u8>()
        .map_err(|_| format!("{label} must be 0-255"))
}

pub(super) fn preview_theme_selection(app: &mut TuiApp, runtime: &TuiRuntime, selected: usize) {
    let saved = runtime.theme();
    app.theme_preview = if let Some(preset) = THEME_PRESETS.get(selected).copied() {
        Some(ThemeSettings::preset(preset))
    } else if saved.name == "custom" {
        Some(saved)
    } else {
        None
    };
}

pub(super) fn checked_or_focused_indices(checked: &[bool], selected: usize) -> Vec<usize> {
    let indices = checked
        .iter()
        .enumerate()
        .filter_map(|(index, checked)| checked.then_some(index))
        .collect::<Vec<_>>();
    if indices.is_empty() {
        vec![selected]
    } else {
        indices
    }
}
