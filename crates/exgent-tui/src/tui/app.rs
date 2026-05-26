use std::io::{self, IsTerminal};

use crossterm::event::{self, Event, KeyEventKind};
use exgent_core::{tr, AppRuntimeHost, MessageId};

use super::auth_flow::run_subscription_auth;
use super::composer_input::handle_paste;
use super::input::handle_key;
use super::prompt::run_prompt;
use super::render::render;
use super::state::*;
use super::terminal::{enter_terminal, handle_resize, TerminalRestoreGuard, TuiTerminal};

type TuiRuntime = AppRuntimeHost;

pub(super) fn run(runtime: &mut TuiRuntime) -> io::Result<()> {
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        return super::run_plain(runtime);
    }

    let mut terminal = enter_terminal()?;
    let _guard = TerminalRestoreGuard;
    let mut app = TuiApp::new(runtime);
    app.push_welcome(tr(app.locale, MessageId::WelcomeNote));

    loop {
        draw(&mut terminal, &mut app, runtime)?;
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                match handle_key(&mut app, runtime, key) {
                    UiAction::None => {}
                    UiAction::Quit => return Ok(()),
                    UiAction::RunPrompt { prompt, images } => {
                        run_prompt(runtime, &mut app, &mut terminal, prompt, images)?;
                    }
                    UiAction::RunSubscriptionAuth(provider) => {
                        run_subscription_auth(runtime, &mut app, &mut terminal, provider)?;
                    }
                }
            }
            Event::Resize(width, height) => {
                handle_resize(&mut terminal, width, height)?;
            }
            Event::Paste(value) => {
                handle_paste(&mut app, &value);
            }
            _ => {}
        }
    }
}

pub(super) fn draw(
    terminal: &mut TuiTerminal,
    app: &mut TuiApp,
    runtime: &TuiRuntime,
) -> io::Result<()> {
    app.refresh_status(runtime);
    terminal.draw(|frame| render(frame, app))?;
    Ok(())
}
