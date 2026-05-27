use std::io;

use exgent_core::AppRuntimeHost;

mod actions;
mod app;
mod auth_flow;
mod auth_input;
mod clipboard_image;
mod clipboard_text;
mod composer_input;
mod formatting;
mod forms;
mod frame_rate_limiter;
mod input;
mod mouse_input;
mod overlays;
mod plain;
mod plain_auth;
mod plain_events;
mod prompt;
mod render;
mod render_helpers;
mod selection;
mod session_input;
mod settings;
mod settings_actions;
mod settings_input;
mod state;
mod suggestions;
mod terminal;
mod transcript_cache;

use plain::{
    accent, bold, clear_rendered_block, dim, exit_process, finish_inline_terminal, inline_height,
    inline_height_for_lines, inline_terminal, print_fitted_terminal_line,
    print_fitted_terminal_lines, read_cancelable_line, refresh_active_footer, render_active_footer,
    reset_terminal_viewport, selected_or_focused_indices, strip_ansi, thinking, RawModeGuard,
};
use selection::select_with_keys;

type TuiRuntime = AppRuntimeHost;

pub fn run_tui(runtime: &mut TuiRuntime) -> io::Result<()> {
    app::run(runtime)
}

pub fn run_plain(runtime: &mut TuiRuntime) -> io::Result<()> {
    plain::run(runtime)
}
