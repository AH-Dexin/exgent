use std::io;

use exgent_core::{AgentEvent, AgentSessionEvent, AppRuntimeHost};

use super::app::draw;
use super::render::render;
use super::state::TuiApp;
use super::terminal::{drain_resize_events, TuiTerminal};

type TuiRuntime = AppRuntimeHost;

pub(super) fn run_prompt(
    runtime: &mut TuiRuntime,
    app: &mut TuiApp,
    terminal: &mut TuiTerminal,
    prompt: String,
) -> io::Result<()> {
    app.is_running = true;
    app.push_user(prompt.clone());
    app.start_assistant();
    draw(terminal, app, runtime)?;

    // Track whether AgentEvent::Error was already displayed via the event stream.
    // apply_agent_event calls push_error for error events, so we must not call it
    // a second time from the Err return value of run_prompt_events.
    let mut streaming_error = false;
    let result = runtime.run_prompt_events(&prompt, &mut |event| {
        if let AgentSessionEvent::Agent(event) = event {
            if matches!(&event, AgentEvent::Error { .. }) {
                streaming_error = true;
            }
            app.apply_agent_event(event);
        }
        drain_resize_events(terminal);
        let _ = terminal.draw(|frame| render(frame, app));
    });
    if let Err(error) = result {
        if !streaming_error {
            app.push_error(error);
        }
    }

    app.is_running = false;
    app.runtime_activity = None;
    app.refresh_status(runtime);
    Ok(())
}
